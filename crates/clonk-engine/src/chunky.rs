//! ChunkOZoom landscape synthesis: the C++ static-map zoom that turns map
//! cells into the Surface8 pixel plane with chunky material rims
//! (C4Landscape::MapToSurface → TexOZoom → ChunkOZoom → DrawChunk,
//! C4Landscape.cpp:280-510). Every helper here is a line-faithful port —
//! the chunk polygons are what keep cave-roof objects (stalactites)
//! attached, so the fills must be bit-exact.

/// An 8-bit drawing surface with the CSurface8 clip semantics
/// (StdSurface8.h:45-51: SetPix clips, GetPix returns 0 out of bounds).
pub(crate) struct Surface8 {
    width: i32,
    height: i32,
    bytes: Vec<u8>,
    clip_x: i32,
    clip_y: i32,
    clip_x2: i32,
    clip_y2: i32,
}

impl Surface8 {
    pub(crate) fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            bytes: vec![0; (width.max(0) as usize) * (height.max(0) as usize)],
            clip_x: 0,
            clip_y: 0,
            clip_x2: width - 1,
            clip_y2: height - 1,
        }
    }

    pub(crate) fn from_bytes(width: i32, height: i32, bytes: Vec<u8>) -> Self {
        debug_assert_eq!(bytes.len(), width.max(0) as usize * height.max(0) as usize);
        Self {
            width,
            height,
            bytes,
            clip_x: 0,
            clip_y: 0,
            clip_x2: width - 1,
            clip_y2: height - 1,
        }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// CSurface8::Clip (StdSurface8.cpp:79-83): clamp both inclusive
    /// endpoints independently to the surface before later SetPix calls.
    pub(crate) fn clip(&mut self, x: i32, y: i32, x2: i32, y2: i32) {
        if self.width <= 0 || self.height <= 0 {
            self.clip_x = 1;
            self.clip_y = 1;
            self.clip_x2 = 0;
            self.clip_y2 = 0;
            return;
        }
        self.clip_x = x.clamp(0, self.width - 1);
        self.clip_y = y.clamp(0, self.height - 1);
        self.clip_x2 = x2.clamp(0, self.width - 1);
        self.clip_y2 = y2.clamp(0, self.height - 1);
    }

    /// CSurface8::SetPix (StdSurface8.h:45-51): silently clipped.
    pub(crate) fn set_pix(&mut self, x: i32, y: i32, col: u8) {
        if x < self.clip_x || x > self.clip_x2 || y < self.clip_y || y > self.clip_y2 {
            return;
        }
        self.bytes[y as usize * self.width as usize + x as usize] = col;
    }

    /// CSurface8::GetPix (StdSurface8.h:53-57): 0 out of bounds.
    pub(crate) fn get_pix(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return 0;
        }
        self.bytes[y as usize * self.width as usize + x as usize]
    }

    /// CSurface8::Box (StdSurface8.cpp:68-71): inclusive corners, HLines of
    /// clipped SetPix.
    pub(crate) fn box_fill(&mut self, x: i32, y: i32, x2: i32, y2: i32, col: u8) {
        for cy in y..=y2 {
            for cx in x..=x2 {
                self.set_pix(cx, cy, col);
            }
        }
    }
}

/// C4Landscape::ChunkyRandom (C4Landscape.cpp:273-278): deterministic
/// per-chunk jitter from the running offset and MapSeed. Zero range
/// short-circuits BEFORE the offset step — load-bearing for parity.
pub(crate) fn chunky_random(offset: &mut i32, range: i32, map_seed: i32) -> i32 {
    if range == 0 {
        return 0;
    }
    *offset += 3;
    (*offset ^ map_seed) % range
}

const POLYGON_FIX_SHIFT: i32 = 16;

/// One Allegro polygon edge (CPolyEdge, StdSurface8.cpp:242-251).
#[derive(Clone, Copy)]
struct PolyEdge {
    y: i32,
    bottom: i32,
    x: i32,
    dx: i32,
    w: i32,
}

/// fill_edge_structure (StdSurface8.cpp:255-268).
fn fill_edge_structure(mut i1: (i32, i32), mut i2: (i32, i32)) -> PolyEdge {
    if i2.1 < i1.1 {
        std::mem::swap(&mut i1, &mut i2);
    }
    let dx = ((i2.0 - i1.0) << POLYGON_FIX_SHIFT) / (i2.1 - i1.1);
    let mut x = (i1.0 << POLYGON_FIX_SHIFT) + (1 << (POLYGON_FIX_SHIFT - 1)) - 1;
    if dx < 0 {
        x += (dx + (1 << POLYGON_FIX_SHIFT)).min(0);
    }
    PolyEdge {
        y: i1.1,
        bottom: i2.1 - 1,
        x,
        dx,
        w: (dx.abs() - (1 << POLYGON_FIX_SHIFT)).max(0),
    }
}

/// CSurface8::Polygon (StdSurface8.cpp:306-404) — the Allegro scanline
/// rasterizer. The linked lists become index vectors; insertion and
/// bubbling replicate add_edge/remove_edge ordering exactly (equal sort
/// keys: the incoming edge goes BEFORE existing ones).
pub(crate) fn polygon(surface: &mut Surface8, vertices: &[(i32, i32)], col: u8) {
    let mut edges: Vec<PolyEdge> = Vec::with_capacity(vertices.len());
    let mut top = i32::MAX;
    let mut bottom = i32::MIN;

    // Fill the edge table: pairs (v[c], v[c-1]) starting with
    // (v[0], v[last]) (StdSurface8.cpp:330-346).
    let mut i2 = vertices[vertices.len() - 1];
    for &i1 in vertices {
        if i1.1 != i2.1 {
            let edge = fill_edge_structure(i1, i2);
            if edge.bottom >= edge.y {
                top = top.min(edge.y);
                bottom = bottom.max(edge.bottom);
                edges.push(edge);
            }
        }
        i2 = i1;
    }
    if edges.is_empty() {
        return;
    }

    // inactive list sorted by y; add_edge inserts before the first entry
    // with an equal-or-greater key (StdSurface8.cpp:269-292).
    let mut inactive: Vec<usize> = Vec::with_capacity(edges.len());
    for (index, edge) in edges.iter().enumerate() {
        let position = inactive
            .iter()
            .position(|&other| edges[other].y >= edge.y)
            .unwrap_or(inactive.len());
        inactive.insert(position, index);
    }

    let mut active: Vec<usize> = Vec::new();
    for c in top..=bottom {
        // Activate edges starting on this scanline, preserving the
        // sort-by-x insertion order (StdSurface8.cpp:351-359).
        while inactive.first().is_some_and(|&head| edges[head].y == c) {
            let head = inactive.remove(0);
            let key = edges[head].x + edges[head].w / 2;
            let position = active
                .iter()
                .position(|&other| edges[other].x + edges[other].w / 2 >= key)
                .unwrap_or(active.len());
            active.insert(position, head);
        }

        // Draw horizontal segments between edge pairs
        // (StdSurface8.cpp:361-373).
        let mut pair = 0;
        while pair + 1 < active.len() {
            let left = &edges[active[pair]];
            let right = &edges[active[pair + 1]];
            let mut x1 = left.x >> POLYGON_FIX_SHIFT;
            let mut x2 = (right.x + right.w) >> POLYGON_FIX_SHIFT;
            if x1 > x2 {
                std::mem::swap(&mut x1, &mut x2);
            }
            for x in x1..=x2 {
                surface.set_pix(x, c, col);
            }
            pair += 2;
        }

        // Update edges: remove dead ones, step x, bubble left
        // (StdSurface8.cpp:375-399). Iterating a snapshot matches the
        // next-pointer walk: bubbling only moves an edge past
        // already-visited predecessors.
        for &index in active.clone().iter() {
            if c >= edges[index].bottom {
                let position = active
                    .iter()
                    .position(|&other| other == index)
                    .expect("edge still active");
                active.remove(position);
            } else {
                edges[index].x += edges[index].dx;
                let mut position = active
                    .iter()
                    .position(|&other| other == index)
                    .expect("edge still active");
                while position > 0
                    && edges[index].x + edges[index].w / 2
                        < edges[active[position - 1]].x + edges[active[position - 1]].w / 2
                {
                    active.swap(position, position - 1);
                    position -= 1;
                }
            }
        }
    }
}

/// Material MapChunkType (C4Material.h:193-196, compiled from `Shape`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum ChunkShape {
    Flat,
    TopFlat,
    Smooth,
    Rough,
}

impl ChunkShape {
    /// `Shape` ini values 0-3 (C4Material.cpp:181); anything else keeps
    /// the compile default Flat.
    pub(crate) fn from_shape(shape: i32) -> Self {
        match shape {
            1 => Self::TopFlat,
            2 => Self::Smooth,
            3 => Self::Rough,
            _ => Self::Flat,
        }
    }
}

/// The DrawChunk octagon (C4Landscape.cpp:299-311) as vertex pairs.
fn chunk_vertices(
    tx: i32,
    ty: i32,
    wdt: i32,
    hgt: i32,
    shape: ChunkShape,
    mut cro: i32,
    map_seed: i32,
) -> [(i32, i32); 8] {
    let (top_rough, side_rough): (i32, i32) = match shape {
        ChunkShape::Flat | ChunkShape::TopFlat => (0, 1),
        ChunkShape::Smooth => (1, 1),
        ChunkShape::Rough => (1, 2),
    };
    let rx = (wdt / 2).max(1);
    let mut draw = |range: i32| chunky_random(&mut cro, range, map_seed);
    [
        (tx - draw(rx / 2), ty - draw(rx / 2 * top_rough)),
        (tx - draw(rx * side_rough), ty + hgt / 2),
        (tx - draw(rx), ty + hgt + draw(rx)),
        (tx + wdt / 2, ty + hgt + draw(2 * rx)),
        (tx + wdt + draw(rx), ty + hgt + draw(rx)),
        (tx + wdt + draw(rx * side_rough), ty + hgt / 2),
        (tx + wdt + draw(rx / 2), ty - draw(rx / 2 * top_rough)),
        (tx + wdt / 2, ty - draw(rx * top_rough)),
    ]
}

/// C4Landscape::DrawChunk (C4Landscape.cpp:280-313).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_chunk(
    surface: &mut Surface8,
    tx: i32,
    ty: i32,
    wdt: i32,
    hgt: i32,
    col: u8,
    shape: ChunkShape,
    cro: i32,
    map_seed: i32,
) {
    if shape == ChunkShape::Flat {
        surface.box_fill(tx, ty, tx + wdt, ty + hgt, col);
        return;
    }
    let vertices = chunk_vertices(tx, ty, wdt, hgt, shape, cro, map_seed);
    if std::env::var("LC_DUMP_CHUNKS").is_ok()
        && (40..=120).contains(&tx)
        && (250..=300).contains(&ty)
    {
        eprintln!(
            "RCHUNK tx={tx} ty={ty} w={wdt} h={hgt} col={col} shape={shape:?} v={vertices:?}"
        );
    }
    polygon(surface, &vertices, col);
}

/// The DrawSmoothOChunk slope quad (C4Landscape.cpp:315-334): both top
/// draws step the offset before the flip overwrite.
fn smooth_o_chunk_vertices(
    tx: i32,
    ty: i32,
    wdt: i32,
    hgt: i32,
    flip: bool,
    mut cro: i32,
    map_seed: i32,
) -> [(i32, i32); 4] {
    let rx = (wdt / 2).max(1);
    let mut vertices = [
        (tx, ty - chunky_random(&mut cro, rx / 2, map_seed)),
        (tx, ty + hgt),
        (tx + wdt, ty + hgt),
        (tx + wdt, ty - chunky_random(&mut cro, rx / 2, map_seed)),
    ];
    let slope = (tx + wdt / 2, ty + hgt / 3);
    if flip {
        vertices[0] = slope;
    } else {
        vertices[3] = slope;
    }
    vertices
}

/// C4Landscape::DrawSmoothOChunk (C4Landscape.cpp:315-334).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_smooth_o_chunk(
    surface: &mut Surface8,
    tx: i32,
    ty: i32,
    wdt: i32,
    hgt: i32,
    col: u8,
    flip: bool,
    cro: i32,
    map_seed: i32,
) {
    let vertices = smooth_o_chunk_vertices(tx, ty, wdt, hgt, flip, cro, map_seed);
    polygon(surface, &vertices, col);
}

const IFT: u8 = 0x80;

/// C4Landscape::ChunkOZoom (C4Landscape.cpp:336-403) for one texture
/// index over the whole map, drawing into the world surface.
fn chunk_o_zoom(
    world: &mut Surface8,
    map: &Surface8,
    texture: u8,
    shape: ChunkShape,
    zoom: i32,
    map_seed: i32,
) {
    let map_width = map.width;
    let map_height = map.height;
    for map_y in 0..map_height {
        let to_y = map_y * zoom;
        for map_x in 0..map_width {
            let pixel = map.get_pix(map_x, map_y);
            let pixel_below = map.get_pix(map_x, map_y + 1);
            let to_x = map_x * zoom;
            let cro = (map_x << 2) + map_y;
            if pixel & 127 == texture {
                let ift = if pixel >= 128 { IFT } else { 0 };
                draw_chunk(
                    world,
                    to_x,
                    to_y,
                    zoom,
                    zoom,
                    texture + ift,
                    shape,
                    cro,
                    map_seed,
                );
            } else if shape == ChunkShape::Smooth
                && map_y < map_height - 1
                && pixel_below & 127 == texture
            {
                // Slope smoothers per matching left/right neighbor
                // (C4Landscape.cpp:377-398), IFT from that neighbor.
                if map_x > 0 && map.get_pix(map_x - 1, map_y) & 127 == texture {
                    let ift = if map.get_pix(map_x - 1, map_y) >= 128 {
                        IFT
                    } else {
                        0
                    };
                    draw_smooth_o_chunk(
                        world,
                        to_x,
                        to_y,
                        zoom,
                        zoom,
                        texture + ift,
                        false,
                        cro,
                        map_seed,
                    );
                }
                if map_x < map_width - 1 && map.get_pix(map_x + 1, map_y) & 127 == texture {
                    let ift = if map.get_pix(map_x + 1, map_y) >= 128 {
                        IFT
                    } else {
                        0
                    };
                    draw_smooth_o_chunk(
                        world,
                        to_x,
                        to_y,
                        zoom,
                        zoom,
                        texture + ift,
                        true,
                        cro,
                        map_seed,
                    );
                }
            }
        }
    }
}

/// MapToSurface for a full static map (C4Landscape.cpp:405-480): count
/// texture usage, then ChunkOZoom every used texture in ASCENDING index
/// order — later indices overwrite earlier chunks where they overlap.
/// `shapes[index]` is the material MapChunkType per texmap index; None
/// means no material is mapped and the texture draws nothing
/// (C4Landscape.cpp:342-343).
pub(crate) fn synthesize_landscape(
    map_bytes: &[u8],
    map_width: i32,
    map_height: i32,
    zoom: i32,
    map_seed: i32,
    shapes: &[Option<ChunkShape>],
) -> Surface8 {
    let map = Surface8::from_bytes(map_width, map_height, map_bytes.to_vec());
    let mut world = Surface8::new(map_width * zoom, map_height * zoom);

    // GetTexUsage (C4Landscape.cpp:405-422): IFT-stripped index counts.
    let mut used = [false; 128];
    for &byte in map_bytes {
        used[(byte & 127) as usize] = true;
    }

    // TexOZoom (C4Landscape.cpp:424-438): indices 1..C4M_MaxTexIndex.
    for texture in 1..127usize {
        if !used[texture] {
            continue;
        }
        let Some(shape) = shapes.get(texture).copied().flatten() else {
            continue;
        };
        chunk_o_zoom(&mut world, &map, texture as u8, shape, zoom, map_seed);
    }
    world
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_fill_is_inclusive_and_clipped_like_csurface8() {
        // CSurface8::Box draws HLines over the INCLUSIVE corner rect
        // (StdSurface8.cpp:68-71); SetPix clips to the surface
        // (StdSurface8.h:45-51).
        let mut surface = Surface8::new(4, 3);
        surface.box_fill(1, 1, 5, 5, 9);
        assert_eq!(
            surface.into_bytes(),
            vec![
                0, 0, 0, 0, //
                0, 9, 9, 9, //
                0, 9, 9, 9,
            ]
        );
    }

    #[test]
    fn explicit_clip_clamps_inclusive_endpoints_like_csurface8() {
        let mut surface = Surface8::new(6, 4);
        surface.clip(2, 1, 4, 2);
        surface.box_fill(0, 0, 5, 3, 7);
        assert_eq!(
            surface.into_bytes(),
            vec![
                0, 0, 0, 0, 0, 0, //
                0, 0, 7, 7, 7, 0, //
                0, 0, 7, 7, 7, 0, //
                0, 0, 0, 0, 0, 0,
            ]
        );
    }

    #[test]
    fn chunky_random_steps_offset_by_three_and_xors_the_seed() {
        // C4Landscape::ChunkyRandom (C4Landscape.cpp:273-278): zero range
        // returns 0 WITHOUT stepping the offset; otherwise offset += 3
        // first, then (offset ^ MapSeed) % range.
        let mut offset = 4;
        assert_eq!(chunky_random(&mut offset, 5, 10), 3); // (7^10)=13 % 5
        assert_eq!(chunky_random(&mut offset, 0, 10), 0); // offset untouched
        assert_eq!(offset, 7);
        assert_eq!(chunky_random(&mut offset, 5, 10), 0); // (10^10)=0
        assert_eq!(chunky_random(&mut offset, 7, 10), 0); // (13^10)=7 % 7
        assert_eq!(offset, 13);
    }

    fn solid_pixels(surface: &Surface8) -> Vec<(i32, i32)> {
        let mut pixels = Vec::new();
        for y in 0..surface.height {
            for x in 0..surface.width {
                if surface.get_pix(x, y) != 0 {
                    pixels.push((x, y));
                }
            }
        }
        pixels
    }

    fn row_span(pixels: &[(i32, i32)], y: i32) -> Option<(i32, i32)> {
        let xs: Vec<i32> = pixels
            .iter()
            .filter(|(_, py)| *py == y)
            .map(|(px, _)| *px)
            .collect();
        xs.iter().min().copied().zip(xs.iter().max().copied())
    }

    #[test]
    fn polygon_fills_an_axis_aligned_rect_with_allegro_rounding() {
        // The Allegro scanline rasterizer (StdSurface8.cpp:241-404):
        // bottom = y2 - 1 (last row excluded), x starts at
        // (x<<16) + 0x8000 - 1, so a rect (2,1)-(8,5) fills x 2..=8,
        // y 1..=4.
        let mut surface = Surface8::new(12, 8);
        polygon(&mut surface, &[(2, 1), (8, 1), (8, 5), (2, 5)], 9);
        let pixels = solid_pixels(&surface);
        for y in 1..=4 {
            assert_eq!(row_span(&pixels, y), Some((2, 8)), "row {y}");
        }
        assert_eq!(pixels.len(), 7 * 4, "no pixels outside rows 1..=4");
    }

    #[test]
    fn polygon_fills_a_right_triangle_like_allegro() {
        // Hand-stepped through fill_edge_structure/scanline pairing
        // (StdSurface8.cpp:255-268,348-373): triangle (0,0),(4,0),(0,4)
        // narrows one pixel per row, hypotenuse edge x = (4<<16)+0x7fff
        // stepping dx = -65536.
        let mut surface = Surface8::new(8, 8);
        polygon(&mut surface, &[(0, 0), (4, 0), (0, 4)], 9);
        let pixels = solid_pixels(&surface);
        assert_eq!(row_span(&pixels, 0), Some((0, 4)));
        assert_eq!(row_span(&pixels, 1), Some((0, 3)));
        assert_eq!(row_span(&pixels, 2), Some((0, 2)));
        assert_eq!(row_span(&pixels, 3), Some((0, 1)));
        assert_eq!(row_span(&pixels, 4), None, "bottom = y2 - 1");
    }

    #[test]
    fn polygon_widens_steep_negative_edges_by_their_gradient() {
        // Steeper-than-diagonal edges get w = |dx| - 65536 and the
        // negative-dx start correction x += dx + 65536
        // (StdSurface8.cpp:259-266); triangle (4,0),(6,0),(0,3) was
        // hand-stepped: rows are 4..=6, 2..=4, 1..=2.
        let mut surface = Surface8::new(10, 6);
        polygon(&mut surface, &[(4, 0), (6, 0), (0, 3)], 9);
        let pixels = solid_pixels(&surface);
        assert_eq!(row_span(&pixels, 0), Some((4, 6)));
        assert_eq!(row_span(&pixels, 1), Some((2, 4)));
        assert_eq!(row_span(&pixels, 2), Some((1, 2)));
        assert_eq!(row_span(&pixels, 3), None);
    }

    #[test]
    fn rough_chunk_vertices_follow_the_chunky_random_chain() {
        // DrawChunk (C4Landscape.cpp:280-313), Rough: top_rough=1,
        // side_rough=2, rx = max(wdt/2, 1). With MapSeed=0 and cro=0 the
        // draws are offset%range for offsets 3,6,9,...,36 — hand-stepped
        // octagon for a 10x10 chunk at (10,10).
        let vertices = chunk_vertices(10, 10, 10, 10, ChunkShape::Rough, 0, 0);
        assert_eq!(
            vertices,
            [
                (9, 10),  // tx-3%2, ty-6%2
                (1, 15),  // tx-9%10, ty+hgt/2
                (8, 20),  // tx-12%5, ty+hgt+15%5
                (15, 28), // tx+wdt/2, ty+hgt+18%10
                (21, 24), // tx+wdt+21%5, ty+hgt+24%5
                (27, 15), // tx+wdt+27%10, ty+hgt/2
                (20, 9),  // tx+wdt+30%2, ty-33%2
                (15, 9),  // tx+wdt/2, ty-36%5
            ]
        );
    }

    #[test]
    fn top_flat_chunks_never_jitter_the_top_but_still_step_the_offset() {
        // C4M_TopFlat: top_rough=0 — every top ChunkyRandom has range 0,
        // returning 0 WITHOUT stepping the offset (C4Landscape.cpp:275),
        // so the side/bottom draws use a SHORTER offset chain than Smooth.
        let vertices = chunk_vertices(10, 10, 10, 10, ChunkShape::TopFlat, 0, 0);
        assert_eq!(vertices[0].1, 10, "top corners stay flat");
        assert_eq!(vertices[6].1, 10);
        assert_eq!(vertices[7], (15, 10), "top middle stays flat");
        // Stepping draws: 3%2,6%5,9%5,12%5,15%10,18%5,21%5,24%5,27%2.
        assert_eq!(
            vertices,
            [
                (9, 10),
                (9, 15),
                (6, 22),
                (15, 25),
                (23, 21),
                (24, 15),
                (21, 10),
                (15, 10),
            ]
        );
    }

    #[test]
    fn flat_chunks_box_fill_with_inclusive_corners() {
        // C4M_Flat short-circuits to Box(tx, ty, tx+wdt, ty+hgt) —
        // INCLUSIVE corners, so a chunk paints one extra row and column
        // (C4Landscape.cpp:285-287).
        let mut surface = Surface8::new(8, 8);
        draw_chunk(&mut surface, 1, 1, 4, 4, 9, ChunkShape::Flat, 0, 0);
        let pixels = solid_pixels(&surface);
        assert_eq!(row_span(&pixels, 1), Some((1, 5)));
        assert_eq!(row_span(&pixels, 5), Some((1, 5)));
        assert_eq!(row_span(&pixels, 6), None);
    }

    #[test]
    fn smooth_o_chunk_flip_replaces_the_matching_top_corner() {
        // DrawSmoothOChunk (C4Landscape.cpp:315-334): a quad with jittered
        // top corners; flip moves the FIRST vertex to the middle slope
        // point, no-flip the LAST. Both top draws step the offset chain
        // (3%2=1, 6%2=0 with seed 0, cro 0).
        assert_eq!(
            smooth_o_chunk_vertices(10, 10, 10, 10, false, 0, 0),
            [(10, 9), (10, 20), (20, 20), (15, 13)]
        );
        assert_eq!(
            smooth_o_chunk_vertices(10, 10, 10, 10, true, 0, 0),
            [(15, 13), (10, 20), (20, 20), (20, 10)]
        );
    }

    fn shapes_with(entries: &[(usize, ChunkShape)]) -> Vec<Option<ChunkShape>> {
        let mut shapes = vec![None; 128];
        for &(index, shape) in entries {
            shapes[index] = Some(shape);
        }
        shapes
    }

    #[test]
    fn later_texture_indices_overwrite_the_shared_chunk_border() {
        // TexOZoom draws used textures in ascending index order
        // (C4Landscape.cpp:428-434); Flat chunks box INCLUSIVE corners
        // (C4Landscape.cpp:285-287), so the border column between two
        // cells always belongs to the HIGHER texture index, whichever
        // side it is on.
        let shapes = shapes_with(&[(1, ChunkShape::Flat), (2, ChunkShape::Flat)]);
        let world = synthesize_landscape(&[1, 2], 2, 1, 4, 0, &shapes);
        assert_eq!(world.get_pix(3, 0), 1);
        assert_eq!(world.get_pix(4, 0), 2, "index 2 drawn after index 1");
        let world = synthesize_landscape(&[2, 1], 2, 1, 4, 0, &shapes);
        assert_eq!(world.get_pix(4, 0), 2, "later index wins from either side");
        assert_eq!(world.get_pix(5, 0), 1);
    }

    #[test]
    fn ift_map_pixels_draw_chunks_with_the_background_bit() {
        // ChunkOZoom: iIFT = IFT for map bytes >= 128, chunk color =
        // texture index + IFT (C4Landscape.cpp:366-375).
        let shapes = shapes_with(&[(1, ChunkShape::Flat)]);
        let world = synthesize_landscape(&[1 | 0x80], 1, 1, 4, 0, &shapes);
        assert_eq!(world.get_pix(0, 0), 129);
        assert_eq!(world.get_pix(3, 3), 129);
    }

    #[test]
    fn smooth_chunks_bleed_beyond_their_block() {
        // A Smooth chunk's top-middle vertex is ty - ChunkyRandom(rx)
        // (C4Landscape.cpp:311): with seed 0 the cell at map (1,1)
        // (cro=5) reaches one pixel above its block — hand-stepped fill
        // 5..=7 at world row 3. This bulge is what keeps cave-roof
        // objects attached.
        let shapes = shapes_with(&[(2, ChunkShape::Smooth)]);
        let world = synthesize_landscape(&[0, 0, 0, 2, 2, 2], 3, 2, 4, 0, &shapes);
        assert_eq!(world.get_pix(5, 3), 2);
        assert_eq!(world.get_pix(6, 3), 2);
        assert_eq!(world.get_pix(7, 3), 2);
        assert_eq!(world.get_pix(4, 3), 0, "bleed is jitter-shaped, not a row");
    }

    #[test]
    fn smooth_textures_draw_slope_smoothers_into_foreign_cells() {
        // ChunkOZoom smoothers (C4Landscape.cpp:377-398): a non-matching
        // cell whose pixel BELOW matches a Smooth texture gets a slope
        // quad per matching left/right neighbor, IFT taken from that
        // neighbor. Map row 0 = [2|IFT, 0, 2], row 1 all 2: cell (1,0)
        // draws a flip=0 quad colored 130 (left neighbor is IFT) filling
        // (5,0), and a flip=1 quad colored 2 filling (7,0) — both pixels
        // are out of reach of every main chunk (hand-stepped octagons).
        let shapes = shapes_with(&[(2, ChunkShape::Smooth)]);
        let world = synthesize_landscape(&[2 | 0x80, 0, 2, 2, 2, 2], 3, 2, 4, 0, &shapes);
        assert_eq!(world.get_pix(5, 0), 130, "left smoother carries left IFT");
        assert_eq!(world.get_pix(7, 0), 2, "right smoother carries right IFT");
    }

    #[test]
    fn get_pix_returns_zero_out_of_bounds_like_csurface8() {
        // StdSurface8.h:53-57.
        let surface = Surface8::from_bytes(2, 1, vec![7, 8]);
        assert_eq!(surface.get_pix(-1, 0), 0);
        assert_eq!(surface.get_pix(0, 1), 0);
        assert_eq!(surface.get_pix(1, 0), 8);
    }
}
