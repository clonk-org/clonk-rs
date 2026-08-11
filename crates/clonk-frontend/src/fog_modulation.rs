use super::*;

/// CPU counterpart of `CClrModAddMap`'s modulation half. `dwAddClr` is
/// retained nowhere because the native renderer never consumes it either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClrModMap {
    pub(crate) resolution_x: i32,
    pub(crate) resolution_y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) origin_x: i32,
    pub(crate) origin_y: i32,
    pub(crate) fade_transparent: bool,
    pub(crate) cells: Vec<u32>,
}

impl ClrModMap {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reset(
        resolution_x: i32,
        resolution_y: i32,
        width_px: i32,
        height_px: i32,
        target_x: i32,
        target_y: i32,
        output_x: i32,
        output_y: i32,
        background_color: u32,
    ) -> Option<Self> {
        if resolution_x <= 0 || resolution_y <= 0 || width_px <= 0 || height_px <= 0 {
            return None;
        }
        // `%` and `/` deliberately retain Rust/C++ truncation toward zero.
        let align_x = -(target_x % resolution_x);
        let align_y = -(target_y % resolution_y);
        let width = (width_px - align_x + resolution_x - 1) / resolution_x + 1;
        let height = (height_px - align_y + resolution_y - 1) / resolution_y + 1;
        let len = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?;
        let fade_transparent = background_color != 0;
        Some(Self {
            resolution_x,
            resolution_y,
            width,
            height,
            origin_x: output_x + align_x,
            origin_y: output_y + align_y,
            fade_transparent,
            cells: vec![if fade_transparent { 0xffff_ffff } else { 0 }; len],
        })
    }

    pub(crate) fn reduce_modulation(
        &mut self,
        center_x: i32,
        center_y: i32,
        radius1: i32,
        radius2: i32,
    ) {
        let radius1_sq = i64::from(radius1) * i64::from(radius1);
        let radius2_sq = i64::from(radius2) * i64::from(radius2);
        let denominator = radius2_sq - radius1_sq;
        for row in 0..self.height {
            let y = self.origin_y + row * self.resolution_y;
            for column in 0..self.width {
                let x = self.origin_x + column * self.resolution_x;
                let dx = i64::from(x) - i64::from(center_x);
                let dy = i64::from(y) - i64::from(center_y);
                let distance = dx * dx + dy * dy;
                if distance >= radius2_sq {
                    continue;
                }
                let index = (row * self.width + column) as usize;
                if distance < radius1_sq {
                    self.cells[index] = 0x00ff_ffff;
                    continue;
                }
                if denominator == 0 {
                    continue;
                }
                let visibility = ((radius2_sq - distance) * 255 / denominator) as u32;
                if self.fade_transparent {
                    let transparency = (self.cells[index] >> 24).min(255 - visibility);
                    self.cells[index] = 0x00ff_ffff | (transparency << 24);
                } else {
                    let gray = visibility * 0x0001_0101;
                    self.cells[index] = self.cells[index].max(gray);
                }
            }
        }
    }

    pub(crate) fn add_modulation(
        &mut self,
        center_x: i32,
        center_y: i32,
        radius1: i32,
        radius2: i32,
        transparency: u8,
    ) {
        let radius1_sq = i64::from(radius1) * i64::from(radius1);
        let radius2_sq = i64::from(radius2) * i64::from(radius2);
        let denominator = radius2_sq - radius1_sq;
        for row in 0..self.height {
            let y = self.origin_y + row * self.resolution_y;
            for column in 0..self.width {
                let x = self.origin_x + column * self.resolution_x;
                let dx = i64::from(x) - i64::from(center_x);
                let dy = i64::from(y) - i64::from(center_y);
                let distance = dx * dx + dy * dy;
                if distance >= radius2_sq {
                    continue;
                }
                let index = (row * self.width + column) as usize;
                if distance < radius1_sq && transparency == 0 {
                    self.cells[index] = 0;
                    continue;
                }
                if denominator == 0 {
                    continue;
                }
                let falloff = ((radius2_sq - distance) * 255 / denominator).min(255);
                let visibility = (255 - falloff + i64::from(transparency)).min(255) as u32;
                if self.fade_transparent {
                    let alpha = (self.cells[index] >> 24).max(255 - visibility);
                    self.cells[index] = 0x00ff_ffff | (alpha << 24);
                } else {
                    let gray = visibility * 0x0001_0101;
                    self.cells[index] = self.cells[index].min(gray);
                }
            }
        }
    }

    pub(crate) fn get_mod_at(&self, x: i32, y: i32) -> u32 {
        let x = x - self.origin_x;
        let y = y - self.origin_y;
        let column = (x / self.resolution_x).clamp(0, self.width - 1);
        let row = (y / self.resolution_y).clamp(0, self.height - 1);
        let column2 = (column + 1).min(self.width - 1);
        let row2 = (row + 1).min(self.height - 1);
        let at = |column: i32, row: i32| self.cells[(row * self.width + column) as usize];
        let corners = [
            at(column, row),
            at(column2, row),
            at(column, row2),
            at(column2, row2),
        ];
        let local_x = i64::from(x - column * self.resolution_x);
        let local_y = i64::from(y - row * self.resolution_y);
        let width = i64::from(self.resolution_x);
        let height = i64::from(self.resolution_y);
        let mut result = 0u32;
        for channel in 0..4 {
            let shift = channel * 8;
            let c0 = i64::from((corners[0] >> shift) & 0xff);
            let cx = i64::from((corners[1] >> shift) & 0xff) - c0;
            let cy = i64::from((corners[2] >> shift) & 0xff) - c0;
            let corner = i64::from((corners[3] >> shift) & 0xff);
            let mut value = c0 + cx * local_x / width + cy * local_y / height;
            value += local_x * local_y * (corner - value) / (width * height);
            result |= (value.clamp(0, 255) as u32) << shift;
        }
        result
    }
}

#[derive(Clone)]
pub(crate) struct FogDrawContext {
    pub(crate) map: Arc<ClrModMap>,
    pub(crate) zoom: f32,
}

impl FogDrawContext {
    pub(crate) fn modulation_at_point(&self, x: f32, y: f32) -> u32 {
        self.map
            .get_mod_at((x / self.zoom) as i32, (y / self.zoom) as i32)
    }

    fn modulation_at(&self, x: i32, y: i32) -> u32 {
        self.modulation_at_point(x as f32, y as f32)
    }

    fn blit_at(&self, blit: SpriteBlitState, x: i32, y: i32) -> SpriteBlitState {
        blit.with_fog_modulation(FogModulationSample::uniform(self.modulation_at(x, y)))
    }

    pub(crate) fn color_at(&self, color: Color, x: i32, y: i32) -> Color {
        modulate_surface_color(color, self.modulation_at(x, y))
    }

    pub(crate) fn color_at_point(&self, color: Color, x: f32, y: f32) -> Color {
        modulate_surface_color(color, self.modulation_at_point(x, y))
    }
}

/// One native sprite/landscape blit is subdivided along source texture
/// coordinates into chunks no larger than 64 pixels. ClrModMap is sampled at
/// each transformed chunk corner and GL smooth-shades the two strip
/// triangles; it does not call `GetModAt` independently for every fragment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FogColorQuad {
    pub(crate) x: (f32, f32),
    pub(crate) y: (f32, f32),
    pub(crate) modulation: [u32; 4], // top-left, top-right, bottom-left, bottom-right
}

/// ClrModMap values at one GL quad's vertices plus the fragment's triangle
/// weights. Native combines the active object/global modulation with each
/// vertex first, then interpolates; interpolating the raw fog color first can
/// differ by a byte because ModulateClr uses integer `>> 8` arithmetic.
#[derive(Clone, Copy)]
pub(crate) struct FogModulationSample {
    pub(crate) modulation: [u32; 4],
    pub(crate) weights: [f32; 4],
}

impl FogModulationSample {
    fn uniform(modulation: u32) -> Self {
        Self {
            modulation: [modulation; 4],
            weights: [1.0, 0.0, 0.0, 0.0],
        }
    }

    pub(crate) fn interpolate(self) -> u32 {
        interpolate_packed_modulation(self.modulation, self.weights)
    }

    pub(crate) fn combine_with(self, base: u32) -> u32 {
        let [top_left, top_right, bottom_left, bottom_right] = self.modulation;
        interpolate_packed_modulation(
            [
                modulate_c4_colors(base, top_left),
                modulate_c4_colors(base, top_right),
                modulate_c4_colors(base, bottom_left),
                modulate_c4_colors(base, bottom_right),
            ],
            self.weights,
        )
    }

    fn combined_quad_is_nonzero(self, base: u32) -> bool {
        let [top_left, top_right, bottom_left, bottom_right] = self.modulation;
        modulate_c4_colors(base, top_left) != 0
            || modulate_c4_colors(base, top_right) != 0
            || modulate_c4_colors(base, bottom_left) != 0
            || modulate_c4_colors(base, bottom_right) != 0
    }

    /// Legacy OpenGL flat shading uses the final vertex of each triangle in
    /// the `TL, TR, BL, BR` strip: BL for the first triangle and BR for the
    /// second. Preserve the complete quad for MOD2's all-black decision while
    /// selecting that provoking vertex for fragment interpolation.
    pub(crate) fn with_flat_provoking_vertex(mut self) -> Self {
        self.weights = if self.weights[3] > 0.0 {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            [0.0, 0.0, 1.0, 0.0]
        };
        self
    }
}

fn interpolate_packed_modulation(modulation: [u32; 4], weights: [f32; 4]) -> u32 {
    let mut result = 0u32;
    for channel in 0..4 {
        let shift = channel * 8;
        let mut value = 0.0f32;
        value += ((modulation[0] >> shift) & 0xff) as f32 * weights[0];
        value += ((modulation[1] >> shift) & 0xff) as f32 * weights[1];
        value += ((modulation[2] >> shift) & 0xff) as f32 * weights[2];
        value += ((modulation[3] >> shift) & 0xff) as f32 * weights[3];
        let value = value.round().clamp(0.0, 255.0) as u32;
        result |= value << shift;
    }
    result
}

pub(crate) struct FogSpriteSampler {
    pub(crate) source_width: f32,
    pub(crate) source_height: f32,
    pub(crate) columns: usize,
    pub(crate) x_ranges: Vec<(f32, f32)>,
    pub(crate) y_ranges: Vec<(f32, f32)>,
    pub(crate) quads: Vec<FogColorQuad>,
}

/// One axis of a rasterized fog sample. A blit reuses the same horizontal
/// coordinate for every row and the same vertical coordinate for every
/// column; resolving the source chunk once per axis avoids searching the
/// chunk lists for every fragment while retaining the exact GL triangle
/// weights.
#[derive(Clone, Copy)]
pub(crate) struct FogAxisSample {
    chunk: usize,
    offset: f32,
}

pub(crate) fn interpolate_quad_color(colors: [Color; 4], weights: [f32; 4]) -> Color {
    let channel = |select: fn(Color) -> u8| {
        colors
            .iter()
            .copied()
            .zip(weights)
            .map(|(color, weight)| f32::from(select(color)) * weight)
            .sum::<f32>()
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::new(
        channel(|color| color.r),
        channel(|color| color.g),
        channel(|color| color.b),
        channel(|color| color.a),
    )
}

impl FogSpriteSampler {
    pub(crate) fn axis_ranges(
        origin: f32,
        extent: f32,
        chunk_size: f32,
        flipped: bool,
    ) -> Vec<(f32, f32)> {
        let mut ranges = Vec::new();
        let end = origin + extent;
        let mut position = origin;
        while position < end {
            let mut next = ((position / chunk_size).floor() + 1.0) * chunk_size;
            if next <= position {
                // At very large f32 source coordinates a 64px increment can
                // round back to the same value. Finish the remaining range
                // instead of allowing malformed script geometry to loop.
                next = end;
            }
            let next = next.min(end);
            if next <= position {
                break;
            }
            let (mut local_start, mut local_end) = (position - origin, next - origin);
            if flipped {
                (local_start, local_end) = (extent - local_end, extent - local_start);
            }
            ranges.push((local_start, local_end));
            position = next;
        }
        ranges.sort_by(|left, right| left.0.total_cmp(&right.0));
        ranges
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        fog: &FogDrawContext,
        dest: (f32, f32, f32, f32),
        source: (f32, f32, f32, f32),
        image_size: (u32, u32),
        flip_x: bool,
        transform: impl Fn(f32, f32) -> (f32, f32),
    ) -> Option<Self> {
        let chunk_size = cpp_tex_size(image_size.0, image_size.1).min(64) as f32;
        Self::new_with_chunks(
            fog,
            dest,
            source,
            (chunk_size, chunk_size),
            flip_x,
            transform,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_chunks(
        fog: &FogDrawContext,
        dest: (f32, f32, f32, f32),
        source: (f32, f32, f32, f32),
        chunk_size: (f32, f32),
        flip_x: bool,
        transform: impl Fn(f32, f32) -> (f32, f32),
    ) -> Option<Self> {
        let (dest_x, dest_y, dest_width, dest_height) = dest;
        let (source_x, source_y, source_width, source_height) = source;
        if ![
            dest_x,
            dest_y,
            dest_width,
            dest_height,
            source_x,
            source_y,
            source_width,
            source_height,
        ]
        .iter()
        .all(|value| value.is_finite())
            || dest_width <= 0.0
            || dest_height <= 0.0
            || source_width <= 0.0
            || source_height <= 0.0
            || !chunk_size.0.is_finite()
            || !chunk_size.1.is_finite()
            || chunk_size.0 <= 0.0
            || chunk_size.1 <= 0.0
        {
            return None;
        }
        let Some(estimated_columns) =
            ((source_width / chunk_size.0).ceil() as usize).checked_add(2)
        else {
            return None;
        };
        let Some(estimated_rows) = ((source_height / chunk_size.1).ceil() as usize).checked_add(2)
        else {
            return None;
        };
        if estimated_columns
            .checked_mul(estimated_rows)
            .is_none_or(|chunks| chunks > 1_000_000)
        {
            return None;
        }
        let x_ranges = Self::axis_ranges(source_x, source_width, chunk_size.0, flip_x);
        let y_ranges = Self::axis_ranges(source_y, source_height, chunk_size.1, false);
        if x_ranges.is_empty() || y_ranges.is_empty() {
            return None;
        }
        let mut quads = Vec::with_capacity(x_ranges.len() * y_ranges.len());
        for &(top, bottom) in &y_ranges {
            for &(left, right) in &x_ranges {
                let target = |local_x: f32, local_y: f32| {
                    transform(
                        dest_x + local_x / source_width * dest_width,
                        dest_y + local_y / source_height * dest_height,
                    )
                };
                let points = [
                    target(left, top),
                    target(right, top),
                    target(left, bottom),
                    target(right, bottom),
                ];
                if !points.iter().all(|(x, y)| x.is_finite() && y.is_finite()) {
                    return None;
                }
                quads.push(FogColorQuad {
                    x: (left, right),
                    y: (top, bottom),
                    modulation: points.map(|(x, y)| fog.modulation_at_point(x, y)),
                });
            }
        }
        Some(Self {
            source_width,
            source_height,
            columns: x_ranges.len(),
            x_ranges,
            y_ranges,
            quads,
        })
    }

    fn quad_and_weights(&self, normalized_x: f32, normalized_y: f32) -> (FogColorQuad, [f32; 4]) {
        let x = Self::axis_sample(&self.x_ranges, self.source_width, normalized_x);
        let y = Self::axis_sample(&self.y_ranges, self.source_height, normalized_y);
        self.quad_and_weights_for_axes(x, y)
    }

    fn axis_sample(ranges: &[(f32, f32)], source_extent: f32, normalized: f32) -> FogAxisSample {
        let local = normalized.clamp(0.0, 1.0) * source_extent;
        let chunk = ranges
            .iter()
            .position(|range| local < range.1)
            .unwrap_or(ranges.len() - 1);
        let range = ranges[chunk];
        FogAxisSample {
            chunk,
            offset: ((local - range.0) / (range.1 - range.0)).clamp(0.0, 1.0),
        }
    }

    pub(crate) fn raster_axes(
        &self,
        width: u32,
        height: u32,
    ) -> (Vec<FogAxisSample>, Vec<FogAxisSample>) {
        self.raster_axes_with_destination_offset(width, height, 0.0, 0.0)
    }

    pub(crate) fn raster_axes_with_destination_offset(
        &self,
        width: u32,
        height: u32,
        offset_x: f32,
        offset_y: f32,
    ) -> (Vec<FogAxisSample>, Vec<FogAxisSample>) {
        let width_f = width as f32;
        let height_f = height as f32;
        let x = (0..width)
            .map(|position| {
                Self::axis_sample(
                    &self.x_ranges,
                    self.source_width,
                    (position as f32 + 0.5 - offset_x) / width_f,
                )
            })
            .collect();
        let y = (0..height)
            .map(|position| {
                Self::axis_sample(
                    &self.y_ranges,
                    self.source_height,
                    (position as f32 + 0.5 - offset_y) / height_f,
                )
            })
            .collect();
        (x, y)
    }

    pub(crate) fn quad_and_weights_for_axes(
        &self,
        x: FogAxisSample,
        y: FogAxisSample,
    ) -> (FogColorQuad, [f32; 4]) {
        let quad = self.quads[y.chunk * self.columns + x.chunk];
        let u = x.offset;
        let v = y.offset;
        let weights = if u + v <= 1.0 {
            [1.0 - u - v, u, v, 0.0]
        } else {
            [0.0, 1.0 - v, 1.0 - u, u + v - 1.0]
        };
        (quad, weights)
    }

    pub(crate) fn modulation_at(&self, normalized_x: f32, normalized_y: f32) -> u32 {
        self.modulation_sample(normalized_x, normalized_y)
            .interpolate()
    }

    pub(crate) fn modulation_sample(
        &self,
        normalized_x: f32,
        normalized_y: f32,
    ) -> FogModulationSample {
        let (quad, weights) = self.quad_and_weights(normalized_x, normalized_y);
        FogModulationSample {
            modulation: quad.modulation,
            weights,
        }
    }

    pub(crate) fn modulation_sample_for_axes(
        &self,
        x: FogAxisSample,
        y: FogAxisSample,
    ) -> FogModulationSample {
        let (quad, weights) = self.quad_and_weights_for_axes(x, y);
        FogModulationSample {
            modulation: quad.modulation,
            weights,
        }
    }

    fn color_at(&self, color: Color, normalized_x: f32, normalized_y: f32) -> Color {
        let (quad, weights) = self.quad_and_weights(normalized_x, normalized_y);
        interpolate_quad_color(
            quad.modulation
                .map(|modulation| modulate_surface_color(color, modulation)),
            weights,
        )
    }

    pub(crate) fn color_at_axes(&self, color: Color, x: FogAxisSample, y: FogAxisSample) -> Color {
        let (quad, weights) = self.quad_and_weights_for_axes(x, y);
        interpolate_quad_color(
            quad.modulation
                .map(|modulation| modulate_surface_color(color, modulation)),
            weights,
        )
    }

    fn vertical_color_at(
        &self,
        normalized_x: f32,
        normalized_y: f32,
        color_at_y: impl Fn(f32) -> Color,
    ) -> Color {
        let (quad, weights) = self.quad_and_weights(normalized_x, normalized_y);
        let top = color_at_y(quad.y.0 / self.source_height);
        let bottom = color_at_y(quad.y.1 / self.source_height);
        interpolate_quad_color(
            [
                modulate_surface_color(top, quad.modulation[0]),
                modulate_surface_color(top, quad.modulation[1]),
                modulate_surface_color(bottom, quad.modulation[2]),
                modulate_surface_color(bottom, quad.modulation[3]),
            ],
            weights,
        )
    }

    pub(crate) fn vertical_color_at_axes(
        &self,
        x: FogAxisSample,
        y: FogAxisSample,
        color_at_y: impl Fn(f32) -> Color,
    ) -> Color {
        let (quad, weights) = self.quad_and_weights_for_axes(x, y);
        let top = color_at_y(quad.y.0 / self.source_height);
        let bottom = color_at_y(quad.y.1 / self.source_height);
        interpolate_quad_color(
            [
                modulate_surface_color(top, quad.modulation[0]),
                modulate_surface_color(top, quad.modulation[1]),
                modulate_surface_color(bottom, quad.modulation[2]),
                modulate_surface_color(bottom, quad.modulation[3]),
            ],
            weights,
        )
    }

    pub(crate) fn normalized_vertical_color_at_axes(
        &self,
        x: FogAxisSample,
        y: FogAxisSample,
        color_at_y: impl Fn(f32) -> Color,
    ) -> Color {
        let (quad, _) = self.quad_and_weights_for_axes(x, y);
        let top = color_at_y(quad.y.0 / self.source_height);
        let bottom = color_at_y(quad.y.1 / self.source_height);
        normalize_quad_colors([
            modulate_surface_color(top, quad.modulation[0]),
            modulate_surface_color(bottom, quad.modulation[2]),
            modulate_surface_color(bottom, quad.modulation[3]),
            modulate_surface_color(top, quad.modulation[1]),
        ])
    }

    pub(crate) fn blit_at(
        &self,
        blit: SpriteBlitState,
        normalized_x: f32,
        normalized_y: f32,
    ) -> SpriteBlitState {
        blit.with_fog_modulation(self.modulation_sample(normalized_x, normalized_y))
    }

    pub(crate) fn blit_at_axes(
        &self,
        blit: SpriteBlitState,
        x: FogAxisSample,
        y: FogAxisSample,
    ) -> SpriteBlitState {
        blit.with_fog_modulation(self.modulation_sample_for_axes(x, y))
    }
}

pub(crate) fn fog_sprite_blit_at(
    sampler: Option<&FogSpriteSampler>,
    fog: Option<&FogDrawContext>,
    blit: SpriteBlitState,
    normalized_x: f32,
    normalized_y: f32,
    target_x: i32,
    target_y: i32,
) -> SpriteBlitState {
    if let Some(sampler) = sampler {
        sampler.blit_at(blit, normalized_x, normalized_y)
    } else if let Some(fog) = fog {
        // A malformed/projective quad can fail sampler construction. Retain
        // visibility rather than dropping modulation for the complete draw.
        fog.blit_at(blit, target_x, target_y)
    } else {
        blit
    }
}

pub(crate) fn blend_prepared_sprite_fragment(
    surface: &mut Surface,
    x: u32,
    y: u32,
    source: PreparedSpriteFragment,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    blend_prepared_sprite_fragment_target(surface, x, y, source, blit, gamma);
}

pub(crate) fn blend_prepared_sprite_fragment_target<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    x: u32,
    y: u32,
    source: PreparedSpriteFragment,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    if !surface.is_gpu_command_capture_active() {
        // The byte-exact software oracle, shared by owned surfaces and
        // borrowed native overlay targets.
        let Some(destination) = surface.get_pixel(x, y) else {
            return;
        };
        let output = composite_sprite_fragment(source, destination, blit, gamma);
        let _ = surface.set_pixel(x, y, output);
        return;
    }
    if let PreparedSpriteFragment::Layers { base, overlay } = source {
        blend_prepared_sprite_fragment_target(surface, x, y, base.into_fragment(), blit, gamma);
        blend_prepared_sprite_fragment_target(surface, x, y, overlay.into_fragment(), blit, gamma);
        return;
    }
    let source = match source {
        PreparedSpriteFragment::Legacy(color) => [
            f32::from(color.r),
            f32::from(color.g),
            f32::from(color.b),
            f32::from(color.a),
        ],
        PreparedSpriteFragment::Shader { rgb, alpha } => [rgb[0], rgb[1], rgb[2], alpha],
        PreparedSpriteFragment::Layers { .. } => unreachable!("layers were split above"),
    };
    let _ = if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
        surface.blend_fragment_additive(x, y, source, gamma)
    } else {
        surface.blend_fragment_over(x, y, source, gamma)
    };
}

fn blend_prepared_sprite_fragment_normal(
    surface: &mut Surface,
    x: u32,
    y: u32,
    source: PreparedSpriteFragment,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    blend_prepared_sprite_fragment(surface, x, y, source, SpriteBlitState::normal(), gamma);
}

/// CPU scratch target for text that will become a straight-alpha texture.
/// RGB stays premultiplied while layers are accumulated; the upload helper
/// below unpremultiplies it exactly once. Final CStdGL targets must keep using
/// [`SurfaceDrawTarget::blend_fragment`]'s non-separate framebuffer equation.
pub(crate) struct PremultipliedTextLayer<'a> {
    surface: &'a mut Surface,
}

impl<'a> PremultipliedTextLayer<'a> {
    pub(crate) fn new(surface: &'a mut Surface) -> Self {
        Self { surface }
    }
}

impl SurfaceDrawTarget for PremultipliedTextLayer<'_> {
    fn width(&self) -> u32 {
        self.surface.width()
    }

    fn height(&self) -> u32 {
        self.surface.height()
    }

    fn clip(&self) -> Option<SurfaceRect> {
        self.surface.clip()
    }

    fn set_clip(&mut self, clip: SurfaceRect) {
        self.surface.set_clip(clip);
    }

    fn clear_clip(&mut self) {
        self.surface.clear_clip();
    }

    fn get_pixel(&self, x: u32, y: u32) -> Option<Color> {
        self.surface.get_pixel(x, y)
    }

    fn set_pixel(
        &mut self,
        x: u32,
        y: u32,
        color: Color,
    ) -> Result<(), clonk_graphics::SurfaceError> {
        self.surface.set_pixel(x, y, color)
    }

    fn blend_fragment(
        &mut self,
        x: u32,
        y: u32,
        source: [f32; 4],
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<(), clonk_graphics::SurfaceError> {
        let Some(destination) = self.get_pixel(x, y) else {
            return Ok(());
        };
        let opacity = (source[3] / 255.0).clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return Ok(());
        }
        let channel = |channel, value: f32, destination: u8| {
            let value = gamma.map_or(value, |ramp| ramp.sample_channel_float(channel, value));
            (value * opacity + f32::from(destination) * (1.0 - opacity))
                .round()
                .clamp(0.0, 255.0) as u8
        };
        self.set_pixel(
            x,
            y,
            Color::new(
                channel(
                    clonk_graphics::gamma::GammaChannel::Red,
                    source[0],
                    destination.r,
                ),
                channel(
                    clonk_graphics::gamma::GammaChannel::Green,
                    source[1],
                    destination.g,
                ),
                channel(
                    clonk_graphics::gamma::GammaChannel::Blue,
                    source[2],
                    destination.b,
                ),
                (source[3].clamp(0.0, 255.0) + f32::from(destination.a) * (1.0 - opacity))
                    .round()
                    .clamp(0.0, 255.0) as u8,
            ),
        )
    }
}

pub(crate) fn retained_straight_alpha_text_image(surface: &Surface) -> ImageData {
    let mut pixels = surface.pixels().to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
    clonk_fonts::retained_font_image(surface.width(), surface.height(), pixels)
}

/// TextOut submits one blit per rendered glyph while ClrModMap is active.
/// Preserve those glyph-local modulation vertices for world-space cursor
/// labels instead of applying one fog sample to the complete line.
pub(crate) fn draw_fogged_cursor_text_line(
    surface: &mut Surface,
    font: &hud::HudFont<'_>,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
    gamma: Option<&clonk_graphics::GammaRamp>,
    renderer_config: AdvancedRendererConfig,
    fog: &FogDrawContext,
) {
    let packed_color = (u32::from(255 - color.a) << 24)
        | (u32::from(color.r) << 16)
        | (u32::from(color.g) << 8)
        | u32::from(color.b);
    let base_blit = SpriteBlitState {
        mode: 0,
        modulation: Some(packed_color),
        fog_modulation: None,
        renderer_config,
    };

    match font {
        hud::HudFont::Clonk(font) => {
            let mut pen_x = x - font.measure(text, false).0 / 2;
            for character in text.chars() {
                if character < ' ' {
                    continue;
                }
                let cell = font.rendered_glyph(character);
                if let Some(cell) = cell {
                    let width = cell.width.max(0) as usize;
                    let height = font.cell_height.max(0) as usize;
                    let retained = surface.is_gpu_scene_capture_active().then(|| {
                        clonk_fonts::retained_font_image(
                            width as u32,
                            height as u32,
                            cell.pixels
                                .iter()
                                .flat_map(|color| [color.r, color.g, color.b, color.a])
                                .collect(),
                        )
                    });
                    if retained.as_ref().is_some_and(|image| {
                        capture_gpu_sprite(
                            surface,
                            (pen_x as f32, y as f32, width as f32, height as f32),
                            (pen_x as f32, y as f32, width as f32, height as f32),
                            &GraphicsTransform::identity(),
                            image,
                            None,
                            FloatSourceRect {
                                x: 0.0,
                                y: 0.0,
                                width: width as f32,
                                height: height as f32,
                            },
                            false,
                            None,
                            base_blit,
                            gamma,
                            Some(fog),
                            GpuSampler::Linear,
                            false,
                        )
                    }) {
                        pen_x = pen_x
                            .saturating_add(cell.width)
                            .saturating_add(font.h_space);
                        continue;
                    }
                    let sampler = FogSpriteSampler::new(
                        fog,
                        (pen_x as f32, y as f32, width as f32, height as f32),
                        (0.0, 0.0, width as f32, height as f32),
                        (width as u32, height as u32),
                        false,
                        |x, y| (x, y),
                    );
                    for row in 0..height {
                        for column in 0..width {
                            let Some(&source_color) = cell
                                .pixels
                                .get(row.saturating_mul(width).saturating_add(column))
                            else {
                                continue;
                            };
                            if source_color.a == 0 {
                                continue;
                            }
                            let target_x = pen_x.saturating_add(column as i32);
                            let target_y = y.saturating_add(row as i32);
                            let pixel_blit = fog_sprite_blit_at(
                                sampler.as_ref(),
                                Some(fog),
                                base_blit,
                                (column as f32 + 0.5) / width.max(1) as f32,
                                (row as f32 + 0.5) / height.max(1) as f32,
                                target_x,
                                target_y,
                            );
                            let source =
                                prepare_sprite_fragment(source_color, None, None, pixel_blit);
                            if source.alpha() == 0.0 {
                                continue;
                            }
                            let (Ok(target_x), Ok(target_y)) =
                                (u32::try_from(target_x), u32::try_from(target_y))
                            else {
                                continue;
                            };
                            blend_prepared_sprite_fragment_normal(
                                surface, target_x, target_y, source, gamma,
                            );
                        }
                    }
                }
                pen_x = pen_x
                    .saturating_add(cell.map_or(0, |cell| cell.width))
                    .saturating_add(font.h_space);
            }
        }
        hud::HudFont::Fallback(fallback) => {
            // The fallback has no exposed glyph atlas. Rasterize the line to
            // a transparent white source and still split it into <=64px fog
            // chunks, so modulation remains spatial rather than line-wide.
            let width = font.text_width(text).max(1) as u32;
            let height = font.line_height().max(1) as u32;
            let origin_x = x - width as i32 / 2;
            let mut source_surface = Surface::new(width, height, PixelFormat::Rgba8888);
            fallback.draw_text(
                &mut source_surface,
                0.0,
                0.0,
                text,
                14.0,
                Color::opaque(255, 255, 255),
            );
            if surface.is_gpu_scene_capture_active() {
                let image = retained_straight_alpha_text_image(&source_surface);
                if capture_gpu_sprite(
                    surface,
                    (origin_x as f32, y as f32, width as f32, height as f32),
                    (origin_x as f32, y as f32, width as f32, height as f32),
                    &GraphicsTransform::identity(),
                    &image,
                    None,
                    FloatSourceRect {
                        x: 0.0,
                        y: 0.0,
                        width: width as f32,
                        height: height as f32,
                    },
                    false,
                    None,
                    base_blit,
                    gamma,
                    Some(fog),
                    GpuSampler::Linear,
                    false,
                ) {
                    return;
                }
            }
            let sampler = FogSpriteSampler::new(
                fog,
                (origin_x as f32, y as f32, width as f32, height as f32),
                (0.0, 0.0, width as f32, height as f32),
                (width, height),
                false,
                |x, y| (x, y),
            );
            for row in 0..height {
                for column in 0..width {
                    let Some(mut source_color) = source_surface.get_pixel(column, row) else {
                        continue;
                    };
                    if source_color.a == 0 {
                        continue;
                    }
                    // Drawing onto transparent black stores coverage in both
                    // RGB and alpha. Recover a straight-alpha white glyph.
                    let alpha = u32::from(source_color.a);
                    let unpremultiply = |channel: u8| {
                        ((u32::from(channel) * 255 + alpha / 2) / alpha).min(255) as u8
                    };
                    source_color.r = unpremultiply(source_color.r);
                    source_color.g = unpremultiply(source_color.g);
                    source_color.b = unpremultiply(source_color.b);
                    let target_x = origin_x.saturating_add(column as i32);
                    let target_y = y.saturating_add(row as i32);
                    let pixel_blit = fog_sprite_blit_at(
                        sampler.as_ref(),
                        Some(fog),
                        base_blit,
                        (column as f32 + 0.5) / width as f32,
                        (row as f32 + 0.5) / height as f32,
                        target_x,
                        target_y,
                    );
                    let source = prepare_sprite_fragment(source_color, None, None, pixel_blit);
                    if source.alpha() == 0.0 {
                        continue;
                    }
                    let (Ok(target_x), Ok(target_y)) =
                        (u32::try_from(target_x), u32::try_from(target_y))
                    else {
                        continue;
                    };
                    blend_prepared_sprite_fragment_normal(
                        surface, target_x, target_y, source, gamma,
                    );
                }
            }
        }
    }
}

/// Markup-aware `TextOut` under ClrModMap. Rasterizing first preserves the
/// font's tag stack across pipe-separated lines; the second pass applies the
/// world-space fog sample to each covered pixel before final gamma blending.
pub(crate) fn draw_fogged_markup_text(
    surface: &mut Surface,
    font: &hud::HudFont<'_>,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
    gamma: Option<&clonk_graphics::GammaRamp>,
    renderer_config: AdvancedRendererConfig,
    fog: &FogDrawContext,
) {
    let (width, measured_height) = font.text_extent_markup(text);
    let raster_height = measured_height.saturating_add(
        font.graphics_line_height()
            .saturating_sub(font.line_height()),
    );
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(raster_height)) else {
        return;
    };
    if width == 0 || height == 0 {
        return;
    }

    let origin_x = x - width as i32 / 2;
    let mut source_surface = Surface::new(width, height, PixelFormat::Rgba8888);
    match font {
        hud::HudFont::Clonk(font) => {
            let mut layer = PremultipliedTextLayer::new(&mut source_surface);
            font.draw_with_gamma_to(
                &mut layer,
                width as i32 / 2,
                0,
                text,
                [color.r, color.g, color.b, color.a],
                clonk_graphics::clonk_font::TextAlign::Center,
                true,
                None,
            );
        }
        hud::HudFont::Fallback(_) => font.draw_markup_with_gamma(
            &mut source_surface,
            width as i32 / 2,
            0,
            text,
            color,
            clonk_graphics::clonk_font::TextAlign::Center,
            None,
        ),
    }
    let base_blit = SpriteBlitState::normal().with_renderer_config(renderer_config);
    if surface.is_gpu_scene_capture_active() {
        let image = retained_straight_alpha_text_image(&source_surface);
        if capture_gpu_sprite(
            surface,
            (origin_x as f32, y as f32, width as f32, height as f32),
            (origin_x as f32, y as f32, width as f32, height as f32),
            &GraphicsTransform::identity(),
            &image,
            None,
            FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: width as f32,
                height: height as f32,
            },
            false,
            None,
            base_blit,
            gamma,
            Some(fog),
            GpuSampler::Linear,
            false,
        ) {
            return;
        }
    }
    let sampler = FogSpriteSampler::new(
        fog,
        (origin_x as f32, y as f32, width as f32, height as f32),
        (0.0, 0.0, width as f32, height as f32),
        (width, height),
        false,
        |x, y| (x, y),
    );
    for row in 0..height {
        for column in 0..width {
            let Some(mut source_color) = source_surface.get_pixel(column, row) else {
                continue;
            };
            if source_color.a == 0 {
                continue;
            }
            // Text was source-over blended onto transparent black. Recover
            // straight-alpha RGB before submitting the final fogged fragment.
            let alpha = u32::from(source_color.a);
            let unpremultiply =
                |channel: u8| ((u32::from(channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            source_color.r = unpremultiply(source_color.r);
            source_color.g = unpremultiply(source_color.g);
            source_color.b = unpremultiply(source_color.b);

            let target_x = origin_x.saturating_add(column as i32);
            let target_y = y.saturating_add(row as i32);
            let pixel_blit = fog_sprite_blit_at(
                sampler.as_ref(),
                Some(fog),
                base_blit,
                (column as f32 + 0.5) / width as f32,
                (row as f32 + 0.5) / height as f32,
                target_x,
                target_y,
            );
            let source = prepare_sprite_fragment(source_color, None, None, pixel_blit);
            if source.alpha() == 0.0 {
                continue;
            }
            let (Ok(target_x), Ok(target_y)) = (u32::try_from(target_x), u32::try_from(target_y))
            else {
                continue;
            };
            blend_prepared_sprite_fragment_normal(surface, target_x, target_y, source, gamma);
        }
    }
}

fn object_is_closed_to_fog(snapshot: &SimulationSnapshot, object: &ObjectSnapshot) -> bool {
    object
        .container
        .and_then(|container| snapshot.object(container))
        .and_then(|container| {
            snapshot
                .definition_closed_containers
                .get(&container.definition_id)
        })
        .is_some_and(|closed| *closed == 1)
}

/// `Graphics.FineFogOfWar` divides the modulation grid by this much.
///
/// C++ resets `ClrModMap` at `Game.C4S.Landscape.FoWRes`
/// (C4Viewport.cpp:1048, default 64 from StdColors.h:379) and then reads it
/// through `GetModAt` only at the four corners of each blit chunk, letting GL
/// smooth-shade between them (StdGL.cpp:729-740).
/// `GraphicsSystem::fog_box_sampler` mirrors that with chunks exactly one cell
/// wide, so the visibility falloff is a piecewise-bilinear approximation of a
/// circle over 64 world pixels, which reads as polygonal seams.
///
/// The falloff has a first-order kink at both radii, so its peak interpolation
/// error falls about linearly with the cell size: measured 72 -> 19 grey levels
/// for 64px -> 16px cells (`fine_fog_cells_track_the_analytic_falloff_far_more_closely`).
/// The cost is quadratic, so 4 is where those meet - see
/// `fine_fog_cell_budget_stays_affordable_at_4k` for the measured budget and
/// `fog_cell_resolution` for why this is presentation-only.
pub(crate) const FINE_FOG_CELL_DIVISOR: i32 = 4;

/// Divisor for the renderer-side fog grid. `false` is the C++-exact grid.
pub(crate) fn fine_fog_cell_divisor(enabled: bool) -> i32 {
    if enabled {
        FINE_FOG_CELL_DIVISOR
    } else {
        1
    }
}

/// Cell size the renderer builds `ClrModMap` with.
///
/// `EnvironmentFrame::fow_resolution` is presentation-only: the sole read in
/// the whole workspace is `build_fog_modulation_map` below, and nothing feeds
/// the chosen cell size back into the simulation or the network protocol, so
/// subdividing it here cannot desync. The snapshot value stays untouched.
///
/// A non-positive `FoWRes` is passed through unchanged so `ClrModMap::reset`
/// keeps rejecting it exactly as it does today.
pub(crate) fn fog_cell_resolution(fow_resolution: i32, cell_divisor: i32) -> i32 {
    if cell_divisor > 1 && fow_resolution > 0 {
        (fow_resolution / cell_divisor).max(1)
    } else {
        fow_resolution
    }
}

pub(crate) fn build_fog_modulation_map(
    snapshot: &SimulationSnapshot,
    owner: i32,
    target_x: i32,
    target_y: i32,
    logical_width: i32,
    logical_height: i32,
) -> Option<ClrModMap> {
    build_fog_modulation_map_with_cell_divisor(
        snapshot,
        owner,
        target_x,
        target_y,
        logical_width,
        logical_height,
        1,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_fog_modulation_map_with_cell_divisor(
    snapshot: &SimulationSnapshot,
    owner: i32,
    target_x: i32,
    target_y: i32,
    logical_width: i32,
    logical_height: i32,
    cell_divisor: i32,
) -> Option<ClrModMap> {
    let player = snapshot
        .players
        .iter()
        .find(|player| player.id == owner && player.fog_of_war)?;
    let resolution = fog_cell_resolution(snapshot.environment.fow_resolution, cell_divisor);
    let mut map = ClrModMap::reset(
        resolution,
        resolution,
        logical_width,
        logical_height,
        target_x,
        target_y,
        0,
        0,
        snapshot.environment.fow_color,
    )?;

    // Real engine snapshots carry the exact runtime link order. Legacy and
    // hand-built presentation fixtures fall back to PlrFoWActualize's owner
    // rule so they remain useful without fabricating a private player list.
    let fow_player = snapshot.fow_players.get(&owner);
    let view_objects = fow_player
        .map(|frame| frame.view_objects.clone())
        .unwrap_or_else(|| {
            snapshot
                .objects
                .iter()
                .filter(|object| {
                    object.status != ObjectStatus::Deleted
                        && object.plr_view_range != 0
                        && (object.owner == owner
                            || !snapshot
                                .players
                                .iter()
                                .any(|player| player.id == object.owner))
                })
                .map(|object| object.id)
                .collect()
        });

    let offset_x = -target_x;
    let offset_y = -target_y;
    let mut has_generators = false;
    for id in &view_objects {
        let Some(object) = snapshot.object(*id) else {
            continue;
        };
        if object_is_closed_to_fog(snapshot, object) {
            continue;
        }
        if object.plr_view_range > 0 {
            map.reduce_modulation(
                object.position.x + offset_x,
                object.position.y + offset_y,
                object.plr_view_range * 2 / 3,
                object.plr_view_range,
            );
        } else {
            has_generators = true;
        }
    }

    if player.view_mode == PLAYER_VIEW_MODE_TARGET {
        if let Some(target) = player
            .view_target
            .or_else(|| fow_player.and_then(|frame| frame.view_target))
            .and_then(|target| snapshot.object(target))
            .filter(|target| !object_is_closed_to_fog(snapshot, target))
        {
            let mut range = target.plr_view_range;
            if range == 0 {
                range = player
                    .cursor
                    .and_then(|cursor| snapshot.object(cursor))
                    .map_or(0, |cursor| cursor.plr_view_range);
            }
            if range == 0 {
                range = 500;
            }
            map.reduce_modulation(
                target.position.x + offset_x,
                target.position.y + offset_y,
                range * 2 / 3,
                range,
            );
        }
    }

    if has_generators {
        for id in view_objects {
            let Some(object) = snapshot.object(id) else {
                continue;
            };
            if object.plr_view_range >= 0 || object_is_closed_to_fog(snapshot, object) {
                continue;
            }
            let radius = -object.plr_view_range;
            map.add_modulation(
                object.position.x + offset_x,
                object.position.y + offset_y,
                radius,
                radius + 200,
                (object.color_modulation >> 24) as u8,
            );
        }
    }
    Some(map)
}

pub(crate) fn c4_color_to_surface(raw: u32) -> Color {
    let [red, green, blue, transparency] = split_c4_color(raw);
    Color::new(red, green, blue, 255 - transparency)
}

pub(crate) fn modulate_surface_color(color: Color, modulation: u32) -> Color {
    let packed = (u32::from(255 - color.a) << 24)
        | (u32::from(color.r) << 16)
        | (u32::from(color.g) << 8)
        | u32::from(color.b);
    c4_color_to_surface(modulate_c4_colors(packed, modulation))
}

/// The ClrByOwner tint passed by C4Object::DrawFace/DrawTopFace to
/// C4DefGraphics::GetBitmap (C4Object.cpp:440-477,2617-2670). This is the
/// live object color, which scripts may change independently of its owner.
pub(crate) fn object_color_by_owner_tint(object: &ObjectSnapshot) -> u32 {
    // C4Surface::SetClr substitutes the legacy blue value 0xff for zero
    // (C4Surface.h:110).
    if object.color == 0 {
        0xff
    } else {
        object.color
    }
}

/// CPU-side `ModulateClr` used to fold global ColorMod into ClrByOwnerClr
/// before the owner texture reaches the shader (StdDDraw2.cpp:773-777).
pub(crate) fn modulate_c4_colors(dst: u32, src: u32) -> u32 {
    let dst = split_c4_color(dst);
    let src = split_c4_color(src);
    let mul = |a: u8, b: u8| (u32::from(a) * u32::from(b)) >> 8;
    let alpha = (u32::from(dst[3]) + u32::from(src[3]) - mul(dst[3], src[3])).min(255);
    (alpha << 24) | (mul(dst[0], src[0]) << 16) | (mul(dst[1], src[1]) << 8) | mul(dst[2], src[2])
}

fn surface_color_to_c4(color: Color) -> u32 {
    (u32::from(255 - color.a) << 24)
        | (u32::from(color.r) << 16)
        | (u32::from(color.g) << 8)
        | u32::from(color.b)
}

fn lighten_c4_color(color: u32) -> u32 {
    let [red, green, blue, transparency] = split_c4_color(color);
    let lighten = |channel: u8| {
        let doubled = (channel & 0x80) | (channel.wrapping_shl(1) & 0xfe);
        if channel & 0x80 != 0 {
            0xff
        } else {
            doubled
        }
    };
    (u32::from(transparency) << 24)
        | (u32::from(lighten(red)) << 16)
        | (u32::from(lighten(green)) << 8)
        | u32::from(lighten(blue))
}

/// `NormalizeColors` from StdColors.h. The legacy operation is a pairwise
/// modulate-and-lighten reduction rather than an arithmetic average.
pub(crate) fn normalize_quad_colors(colors: [Color; 4]) -> Color {
    let [mut top_left, top_right, mut bottom_left, bottom_right] = colors.map(surface_color_to_c4);
    top_left = lighten_c4_color(modulate_c4_colors(top_left, top_right));
    bottom_left = lighten_c4_color(modulate_c4_colors(bottom_left, bottom_right));
    c4_color_to_surface(lighten_c4_color(modulate_c4_colors(top_left, bottom_left)))
}

fn shader_modulate_sample(
    source: [f32; 4],
    modulation: u32,
    mod2: bool,
    renderer_config: AdvancedRendererConfig,
) -> PreparedSpriteFragment {
    let modulation = split_c4_color(modulation);
    if mod2 {
        let channel = |source: f32, modulation: u8| {
            (2.0 * source + 2.0 * f32::from(modulation) - 255.0).clamp(0.0, 255.0)
        };
        PreparedSpriteFragment::Shader {
            rgb: [
                channel(source[0], modulation[0]),
                channel(source[1], modulation[1]),
                channel(source[2], modulation[2]),
            ],
            // LC_MOD2 leaves texture alpha untouched, while the fixed
            // combiner still uses GL_ADD for texture + modulation alpha
            // (StdGL.cpp:490-503,1072-1075).
            alpha: if renderer_config.shader {
                source[3]
            } else {
                (source[3] - f32::from(modulation[3])).max(0.0)
            },
        }
    } else {
        let channel = |source: f32, modulation: u8| source * f32::from(modulation) / 255.0;
        PreparedSpriteFragment::Shader {
            rgb: [
                channel(source[0], modulation[0]),
                channel(source[1], modulation[1]),
                channel(source[2], modulation[2]),
            ],
            // Textures carry normal opacity in Rust, while StdGL adds C4's
            // transparency-alpha modulation to texture transparency.
            alpha: if !renderer_config.shader && renderer_config.no_alpha_add {
                source[3]
            } else {
                (source[3] - f32::from(modulation[3])).max(0.0)
            },
        }
    }
}

fn shader_modulate_fragment(
    source: Color,
    modulation: u32,
    mod2: bool,
    renderer_config: AdvancedRendererConfig,
) -> PreparedSpriteFragment {
    shader_modulate_sample(
        [
            f32::from(source.r),
            f32::from(source.g),
            f32::from(source.b),
            f32::from(source.a),
        ],
        modulation,
        mod2,
        renderer_config,
    )
}

fn prepare_color_by_owner_fragment(
    source: Color,
    mut modulation: u32,
    blit: SpriteBlitState,
) -> PreparedSpriteFragment {
    if let Some(global) = blit.modulation {
        if blit.mode & C4GFXBLIT_CLRSFC_OWNCLR == 0 {
            modulation = modulate_c4_colors(modulation, global);
        }
    }
    let uses_mod2 = blit.mode & C4GFXBLIT_CLRSFC_MOD2 != 0;
    let quad_modulation_is_nonzero = if modulation != 0 {
        blit.fog_modulation.is_none_or(|fog| {
            let any_nonzero = !uses_mod2 || fog.combined_quad_is_nonzero(modulation);
            modulation = fog.combine_with(modulation);
            any_nonzero
        })
    } else {
        false
    };
    // PerformBlt explicitly disables MOD2 for a completely black modulation
    // quad, not independently at each interpolated fragment (StdGL.cpp:471-472).
    let mod2 = uses_mod2 && quad_modulation_is_nonzero;
    shader_modulate_fragment(source, modulation, mod2, blit.renderer_config)
}

pub(crate) fn prepare_sprite_fragment(
    source: Color,
    owner_mask: Option<ColorByOwnerSample>,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
) -> PreparedSpriteFragment {
    if let (Some(ColorByOwnerSample::Overlay(overlay)), Some(modulation)) =
        (owner_mask, owner_color)
    {
        if overlay.a != 0 {
            let base = prepare_sprite_fragment(source, None, None, blit).into_layer();
            let overlay = prepare_color_by_owner_fragment(overlay, modulation, blit).into_layer();
            return PreparedSpriteFragment::Layers { base, overlay };
        }
    }

    if let (Some(ColorByOwnerSample::Scalar(mask)), Some(modulation)) = (owner_mask, owner_color) {
        if mask == 0 {
            return prepare_sprite_fragment(source, None, None, blit);
        }
        // The mask stores the grey ClrByOwner texture intensity. Its main-sfc
        // pixel was cleared when C4Surface::CreateColorByOwner split the image
        // (C4Surface.cpp:288-312).
        return prepare_color_by_owner_fragment(
            Color::new(mask, mask, mask, source.a),
            modulation,
            blit,
        );
    }

    if blit.modulation.is_none() && blit.fog_modulation.is_none() && blit.mode & C4GFXBLIT_MOD2 == 0
    {
        return PreparedSpriteFragment::Legacy(source);
    }

    let mut modulation = blit.modulation.unwrap_or(0x00ff_ffff);
    let uses_mod2 = blit.mode & C4GFXBLIT_MOD2 != 0;
    let quad_modulation_is_nonzero = if modulation != 0 {
        blit.fog_modulation.is_none_or(|fog| {
            let any_nonzero = !uses_mod2 || fog.combined_quad_is_nonzero(modulation);
            modulation = fog.combine_with(modulation);
            any_nonzero
        })
    } else {
        false
    };
    let mod2 = uses_mod2 && quad_modulation_is_nonzero;
    shader_modulate_fragment(source, modulation, mod2, blit.renderer_config)
}

/// Prepare a GL-filtered texture result without quantizing it back to an
/// eight-bit texel. Native filtering precedes both owner-color passes and all
/// shader modulation, so fractional RGBA must survive until framebuffer
/// composition.
pub(crate) fn prepare_filtered_sprite_fragment(
    source: [f32; 4],
    owner_mask: Option<FilteredColorByOwnerSample>,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
) -> PreparedSpriteFragment {
    if let (Some(FilteredColorByOwnerSample::Overlay(overlay)), Some(modulation)) =
        (owner_mask, owner_color)
    {
        if overlay[3] > 0.0 {
            let base = prepare_filtered_sprite_fragment(source, None, None, blit).into_layer();
            let overlay =
                prepare_filtered_color_by_owner_fragment(overlay, modulation, blit).into_layer();
            return PreparedSpriteFragment::Layers { base, overlay };
        }
    }

    if let (Some(FilteredColorByOwnerSample::Scalar(mask)), Some(modulation)) =
        (owner_mask, owner_color)
    {
        if mask <= 0.0 {
            return prepare_filtered_sprite_fragment(source, None, None, blit);
        }
        return prepare_filtered_color_by_owner_fragment(
            [mask, mask, mask, source[3]],
            modulation,
            blit,
        );
    }

    let mut modulation = blit.modulation.unwrap_or(0x00ff_ffff);
    let uses_mod2 = blit.mode & C4GFXBLIT_MOD2 != 0;
    let quad_modulation_is_nonzero = if modulation != 0 {
        blit.fog_modulation.is_none_or(|fog| {
            let any_nonzero = !uses_mod2 || fog.combined_quad_is_nonzero(modulation);
            modulation = fog.combine_with(modulation);
            any_nonzero
        })
    } else {
        false
    };
    let mod2 = uses_mod2 && quad_modulation_is_nonzero;
    shader_modulate_sample(source, modulation, mod2, blit.renderer_config)
}

fn prepare_filtered_color_by_owner_fragment(
    source: [f32; 4],
    mut modulation: u32,
    blit: SpriteBlitState,
) -> PreparedSpriteFragment {
    if let Some(global) = blit.modulation {
        if blit.mode & C4GFXBLIT_CLRSFC_OWNCLR == 0 {
            modulation = modulate_c4_colors(modulation, global);
        }
    }
    let uses_mod2 = blit.mode & C4GFXBLIT_CLRSFC_MOD2 != 0;
    let quad_modulation_is_nonzero = if modulation != 0 {
        blit.fog_modulation.is_none_or(|fog| {
            let any_nonzero = !uses_mod2 || fog.combined_quad_is_nonzero(modulation);
            modulation = fog.combine_with(modulation);
            any_nonzero
        })
    } else {
        false
    };
    shader_modulate_sample(
        source,
        modulation,
        uses_mod2 && quad_modulation_is_nonzero,
        blit.renderer_config,
    )
}

/// Prepares the animated landscape shader's float RGB without quantizing the
/// Liquid.png contribution before global modulation, fog, and gamma.
pub(crate) fn prepare_liquid_animation_fragment(
    source: Color,
    liquid_delta: f32,
    blit: SpriteBlitState,
) -> PreparedSpriteFragment {
    debug_assert_eq!(blit.mode & C4GFXBLIT_MOD2, 0);
    let mut modulation = blit.modulation.unwrap_or(0x00ff_ffff);
    if modulation != 0 {
        if let Some(fog) = blit.fog_modulation {
            modulation = fog.combine_with(modulation);
        }
    }
    let modulation = split_c4_color(modulation);
    let channel = |source: u8, modulation: u8| {
        (f32::from(source) / 255.0 + liquid_delta).clamp(0.0, 1.0) * f32::from(modulation)
    };
    PreparedSpriteFragment::Shader {
        rgb: [
            channel(source.r, modulation[0]),
            channel(source.g, modulation[1]),
            channel(source.b, modulation[2]),
        ],
        alpha: if !blit.renderer_config.shader && blit.renderer_config.no_alpha_add {
            f32::from(source.a)
        } else {
            f32::from(source.a.saturating_sub(modulation[3]))
        },
    }
}

#[cfg(test)]
mod fine_fog_tests {
    use super::*;

    /// C++ resets the map at the raw `Game.C4S.Landscape.FoWRes`
    /// (C4Viewport.cpp:1048); divisor 1 must reproduce that exactly.
    #[test]
    fn fog_cell_resolution_keeps_the_cpp_grid_unless_subdivision_is_requested() {
        assert_eq!(fog_cell_resolution(64, 1), 64);
        assert_eq!(fog_cell_resolution(64, 0), 64);
        assert_eq!(fog_cell_resolution(64, -4), 64);
        assert_eq!(fine_fog_cell_divisor(false), 1);
    }

    #[test]
    fn fine_fog_subdivides_the_grid_and_never_reaches_zero() {
        assert_eq!(fine_fog_cell_divisor(true), FINE_FOG_CELL_DIVISOR);
        assert_eq!(fog_cell_resolution(64, FINE_FOG_CELL_DIVISOR), 16);
        assert_eq!(fog_cell_resolution(96, FINE_FOG_CELL_DIVISOR), 24);
        // A scenario may already ask for a fine grid; `res / divisor` must not
        // reach 0 because `ClrModMap::reset` rejects that outright.
        assert_eq!(fog_cell_resolution(2, FINE_FOG_CELL_DIVISOR), 1);
        assert_eq!(fog_cell_resolution(1, FINE_FOG_CELL_DIVISOR), 1);
        // Non-positive FoWRes stays rejected rather than being rounded up.
        assert_eq!(fog_cell_resolution(0, FINE_FOG_CELL_DIVISOR), 0);
        let rejected = fog_cell_resolution(0, FINE_FOG_CELL_DIVISOR);
        assert!(ClrModMap::reset(rejected, 64, 8, 8, 0, 0, 0, 0, 0).is_none());
    }

    /// Peak deviation of the sampled falloff from the analytic
    /// `(r2^2 - d^2) * 255 / (r2^2 - r1^2)` visibility ramp that
    /// `reduce_modulation` evaluates per cell corner. Bilinear interpolation
    /// between corners is what makes the boundary look polygonal, so this
    /// number *is* the faceting.
    fn peak_falloff_error(resolution: i32, radius1: i32, radius2: i32) -> i64 {
        let extent = radius2 * 3;
        let mut map =
            ClrModMap::reset(resolution, resolution, extent, extent, 0, 0, 0, 0, 0).unwrap();
        let center = extent / 2;
        map.reduce_modulation(center, center, radius1, radius2);
        let radius1_sq = i64::from(radius1) * i64::from(radius1);
        let radius2_sq = i64::from(radius2) * i64::from(radius2);
        let mut peak = 0;
        for y in 0..extent {
            for x in 0..extent {
                let dx = i64::from(x - center);
                let dy = i64::from(y - center);
                let distance = dx * dx + dy * dy;
                let ideal = if distance < radius1_sq {
                    255
                } else if distance >= radius2_sq {
                    0
                } else {
                    (radius2_sq - distance) * 255 / (radius2_sq - radius1_sq)
                };
                let sampled = i64::from(map.get_mod_at(x, y) & 0xff);
                peak = peak.max((sampled - ideal).abs());
            }
        }
        peak
    }

    #[test]
    fn fine_fog_cells_track_the_analytic_falloff_far_more_closely() {
        let coarse = peak_falloff_error(64, 200, 300);
        let fine = peak_falloff_error(fog_cell_resolution(64, FINE_FOG_CELL_DIVISOR), 200, 300);
        // The ramp has a first-order kink at both radii, so the peak error
        // falls roughly linearly with the cell size rather than quadratically:
        // 72 -> 19 grey levels for 64px -> 16px cells.
        assert!(
            fine * 3 < coarse,
            "16px cells must cut the faceting error by more than 3x: coarse {coarse}, fine {fine}"
        );
    }

    /// Both `reduce_modulation` and the box sampler are quadratic in the cell
    /// count, so the divisor has to stay defensible at the worst realistic
    /// viewport: 3840x2160 world pixels at zoom 1. Measured there (release,
    /// M-series): 2135 cells / 75us sampler build at the C++ 64px grid versus
    /// 32776 cells / 1.21ms at 16px. A divisor of 8 would be 131k cells and
    /// ~5ms per sampler, which is not affordable, so pin the budget.
    #[test]
    fn fine_fog_cell_budget_stays_affordable_at_4k() {
        let resolution = fog_cell_resolution(64, FINE_FOG_CELL_DIVISOR);
        let coarse = ClrModMap::reset(64, 64, 3840, 2160, 0, 0, 0, 0, 0).unwrap();
        let fine = ClrModMap::reset(resolution, resolution, 3840, 2160, 0, 0, 0, 0, 0).unwrap();
        assert_eq!(coarse.cells.len(), 2135);
        assert_eq!(fine.cells.len(), 32776);
        assert!(
            fine.cells.len() < 40_000,
            "fine fog grid grew past its 4K budget: {} cells",
            fine.cells.len()
        );
    }

    /// `GraphicsSystem::fog_box_sampler` chunks world blits at
    /// `map.resolution_x`/`resolution_y`, so subdividing the grid is also what
    /// subdivides the interpolated quads the fog boundary is drawn from.
    #[test]
    fn fine_fog_resolution_reaches_the_map_the_box_sampler_chunks_from() {
        let resolution = fog_cell_resolution(64, FINE_FOG_CELL_DIVISOR);
        let map = ClrModMap::reset(resolution, resolution, 256, 256, 0, 0, 0, 0, 0).unwrap();
        assert_eq!((map.resolution_x, map.resolution_y), (16, 16));
    }
}
