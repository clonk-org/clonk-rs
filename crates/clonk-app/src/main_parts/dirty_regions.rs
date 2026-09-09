#[cfg(test)]
use super::RetainedGpuFrameLayer;
use super::{RetainedGpuFrame, RetainedGpuLayerOwner};
use clonk_graphics::{
    ClipperProjection, Color, DamageRegion, GpuCommand, GpuGammaMode, GpuPresentation,
    GpuPrimitiveTopology, GpuTextureId, PaintNode, Rect,
};
use std::collections::HashMap;

/// The projection inputs that make a retained command's coordinates meaningful.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RetainedGpuCoordinateSpace {
    pub(crate) logical_extent: [u32; 2],
    pub(crate) physical_extent: [u32; 2],
    pub(crate) scale_bits: u32,
    pub(crate) crop_top: u32,
    pub(crate) world_zoom_bits: u32,
}

impl RetainedGpuCoordinateSpace {
    fn new(logical_extent: [u32; 2], presentation: GpuPresentation) -> Self {
        Self {
            logical_extent,
            physical_extent: presentation.physical_extent,
            scale_bits: presentation.scale.to_bits(),
            crop_top: presentation.crop_top,
            world_zoom_bits: presentation.world_zoom.to_bits(),
        }
    }
}

/// Resource family used to number otherwise-anonymous retained commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RetainedGpuPaintKind {
    Quad {
        texture: GpuTextureId,
        owner_mask: Option<GpuTextureId>,
    },
    Sprite {
        texture: GpuTextureId,
    },
    Object {
        texture: GpuTextureId,
        owner_texture: Option<GpuTextureId>,
    },
    Landscape {
        base: GpuTextureId,
        liquid_mask: Option<GpuTextureId>,
        liquid: Option<GpuTextureId>,
    },
    Solid {
        topology: RetainedGpuSolidKind,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RetainedGpuSolidKind {
    Triangles,
    Lines,
    Points,
}

/// Stable best-effort identity for one command-level paint atom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RetainedGpuPaintId {
    pub(crate) coordinate_space: RetainedGpuCoordinateSpace,
    pub(crate) kind: RetainedGpuPaintKind,
    pub(crate) bounds: Rect,
    pub(crate) occurrence: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReferencedTextureRevision {
    pub(crate) id: GpuTextureId,
    /// `None` preserves a missing-resource reference as distinct state.
    pub(crate) revision: Option<u64>,
}

/// Exact atom command plus every sampled resource revision it references.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RetainedGpuPaintVisual {
    pub(crate) command: GpuCommand,
    pub(crate) texture_revisions: Vec<ReferencedTextureRevision>,
}

pub(crate) type RetainedGpuPaintNode = PaintNode<RetainedGpuPaintId, RetainedGpuPaintVisual>;

/// Frame-wide state whose change invalidates every physical output pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RetainedGpuFramePaintSignature {
    /// Only the base layer clears the composition target and supplies the
    /// shared gamma resource. Later ordered layers may appear or split without
    /// changing this global signature; their coordinate spaces live in atom
    /// identities instead.
    pub(crate) base: Option<RetainedGpuLayerPaintSignature>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RetainedGpuLayerPaintSignature {
    pub(crate) coordinate_space: RetainedGpuCoordinateSpace,
    pub(crate) clear: Color,
    pub(crate) gamma_revision: u64,
    pub(crate) gamma_mode: GpuGammaMode,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RetainedGpuPaintOwners {
    pub(crate) physical_bounds: Rect,
    pub(crate) signature: RetainedGpuFramePaintSignature,
    pub(crate) nodes: Vec<RetainedGpuPaintNode>,
}

impl RetainedGpuPaintOwners {
    /// Compare two extractions, failing closed to the complete current target
    /// whenever a frame-wide raster or coordinate-space input changed.
    pub(crate) fn damage_from(&self, previous: &Self) -> DamageRegion {
        if self.signature != previous.signature {
            let mut damage = DamageRegion::new(self.physical_bounds);
            damage.add(self.physical_bounds);
            return damage;
        }
        DamageRegion::between(self.physical_bounds, &previous.nodes, &self.nodes)
    }
}

/// Extract command-level paint owners. Only an explicitly isolated semantic
/// layer may absorb conservative line/point bounds beyond its known raster;
/// coincident commands in every other layer remain independent owners.
pub(crate) fn retained_gpu_frame_paint_owners(
    frame: &RetainedGpuFrame,
    raster_halo_owners: &[Rect],
) -> RetainedGpuPaintOwners {
    let signature = RetainedGpuFramePaintSignature {
        base: frame
            .layers
            .first()
            .map(|layer| RetainedGpuLayerPaintSignature {
                coordinate_space: RetainedGpuCoordinateSpace::new(
                    layer.scene.logical_extent,
                    layer.presentation,
                ),
                clear: layer.scene.clear,
                gamma_revision: layer.scene.gamma.revision,
                gamma_mode: layer.scene.gamma_mode,
            }),
    };
    let physical_extent = frame.layers.iter().fold([0_u32; 2], |extent, layer| {
        [
            extent[0].max(layer.presentation.physical_extent[0]),
            extent[1].max(layer.presentation.physical_extent[1]),
        ]
    });
    let physical_bounds = Rect::new(0, 0, physical_extent[0], physical_extent[1]);
    let mut occurrences =
        HashMap::<(RetainedGpuCoordinateSpace, RetainedGpuPaintKind, Rect), usize>::new();
    let mut nodes = Vec::new();
    let mut atoms = Vec::new();

    for layer in &frame.layers {
        let owns_startup_tooltip =
            layer.owner == Some(RetainedGpuLayerOwner::StartupElementTooltip);
        let coordinate_space =
            RetainedGpuCoordinateSpace::new(layer.scene.logical_extent, layer.presentation);
        let texture_revisions = layer
            .scene
            .textures
            .iter()
            .map(|resource| (resource.id, resource.revision))
            .collect::<HashMap<_, _>>();
        for command in &layer.scene.commands {
            split_command_into(command, &mut atoms);
            for atom in atoms.drain(..) {
                let Some(bounds) =
                    physical_command_bounds(&atom, layer.scene.logical_extent, layer.presentation)
                else {
                    continue;
                };
                if owns_startup_tooltip
                    && raster_halo_owners
                        .iter()
                        .copied()
                        .any(|owner| command_raster_halo_is_owned_by(owner, bounds, &atom))
                {
                    continue;
                }
                let kind = paint_kind(&atom);
                let next_occurrence = occurrences
                    .entry((coordinate_space, kind, bounds))
                    .or_default();
                let occurrence = *next_occurrence;
                *next_occurrence = next_occurrence.saturating_add(1);
                let referenced_texture_revisions = referenced_texture_ids(&atom)
                    .into_iter()
                    .map(|id| ReferencedTextureRevision {
                        id,
                        revision: texture_revisions.get(&id).copied(),
                    })
                    .collect();
                nodes.push(PaintNode::new(
                    RetainedGpuPaintId {
                        coordinate_space,
                        kind,
                        bounds,
                        occurrence,
                    },
                    bounds,
                    RetainedGpuPaintVisual {
                        command: atom,
                        texture_revisions: referenced_texture_revisions,
                    },
                ));
            }
        }
    }

    RetainedGpuPaintOwners {
        physical_bounds,
        signature,
        nodes,
    }
}

/// Project a semantic owner's logical bounds through the full target clip and
/// retain only the pixels that reach the physical framebuffer.
pub(crate) fn retained_gpu_physical_bounds(
    logical_bounds: Rect,
    logical_extent: [u32; 2],
    presentation: GpuPresentation,
) -> Option<Rect> {
    if !presentation.scale.is_finite() || presentation.scale <= 0.0 {
        return None;
    }
    let target = Rect::new(
        0,
        0,
        presentation.physical_extent[0],
        presentation.physical_extent[1],
    );
    let logical_target = Rect::new(0, 0, logical_extent[0], logical_extent[1]);
    let logical_bounds = logical_bounds.intersection(logical_target)?;
    let viewport_height = ((logical_extent[1] as f32) * presentation.scale)
        .ceil()
        .clamp(0.0, u32::MAX as f32) as u32;
    let projection = ClipperProjection::new(
        presentation.scale,
        (logical_extent[0], logical_extent[1]),
        viewport_height.saturating_sub(presentation.crop_top),
        logical_target,
    );
    projected_rect(logical_bounds, projection)?.intersection(target)
}

fn split_command_into(command: &GpuCommand, atoms: &mut Vec<GpuCommand>) {
    atoms.clear();
    match command {
        GpuCommand::Quad { .. } | GpuCommand::Landscape { .. } => atoms.push(command.clone()),
        GpuCommand::SpriteBatch {
            texture,
            quads,
            clip,
            blend,
            mod2,
            gamma,
            outer_modulation,
        } => atoms.extend(quads.iter().map(|quad| GpuCommand::SpriteBatch {
            texture: *texture,
            quads: vec![*quad],
            clip: *clip,
            blend: *blend,
            mod2: *mod2,
            gamma: *gamma,
            outer_modulation: *outer_modulation,
        })),
        GpuCommand::ObjectBatch {
            texture,
            owner_texture,
            sprites,
            clip,
            blend,
            gamma,
        } => atoms.extend(sprites.iter().map(|sprite| GpuCommand::ObjectBatch {
            texture: *texture,
            owner_texture: *owner_texture,
            sprites: vec![*sprite],
            clip: *clip,
            blend: *blend,
            gamma: *gamma,
        })),
        GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            clip,
            blend,
            style,
        } => {
            let primitive_size = match topology {
                GpuPrimitiveTopology::TriangleList => 3,
                GpuPrimitiveTopology::LineList => 2,
                GpuPrimitiveTopology::PointList => 1,
            };
            atoms.extend(
                vertices
                    .chunks(primitive_size)
                    .map(|primitive| GpuCommand::Solid {
                        vertices: primitive.to_vec(),
                        topology: *topology,
                        alpha_mode: *alpha_mode,
                        clip: *clip,
                        blend: *blend,
                        style: *style,
                    }),
            );
        }
    }
}

fn paint_kind(command: &GpuCommand) -> RetainedGpuPaintKind {
    match command {
        GpuCommand::Quad {
            texture,
            owner_mask,
            ..
        } => RetainedGpuPaintKind::Quad {
            texture: *texture,
            owner_mask: owner_mask.map(|(id, _)| id),
        },
        GpuCommand::SpriteBatch { texture, .. } => {
            RetainedGpuPaintKind::Sprite { texture: *texture }
        }
        GpuCommand::ObjectBatch {
            texture,
            owner_texture,
            ..
        } => RetainedGpuPaintKind::Object {
            texture: *texture,
            owner_texture: *owner_texture,
        },
        GpuCommand::Landscape {
            base,
            liquid_mask,
            liquid,
            ..
        } => RetainedGpuPaintKind::Landscape {
            base: *base,
            liquid_mask: *liquid_mask,
            liquid: *liquid,
        },
        GpuCommand::Solid { topology, .. } => RetainedGpuPaintKind::Solid {
            topology: match topology {
                GpuPrimitiveTopology::TriangleList => RetainedGpuSolidKind::Triangles,
                GpuPrimitiveTopology::LineList => RetainedGpuSolidKind::Lines,
                GpuPrimitiveTopology::PointList => RetainedGpuSolidKind::Points,
            },
        },
    }
}

fn referenced_texture_ids(command: &GpuCommand) -> Vec<GpuTextureId> {
    let mut ids = Vec::with_capacity(3);
    let mut push = |id| {
        if !ids.contains(&id) {
            ids.push(id);
        }
    };
    match command {
        GpuCommand::Quad {
            texture,
            owner_mask,
            ..
        } => {
            push(*texture);
            owner_mask.iter().for_each(|(id, _)| push(*id));
        }
        GpuCommand::SpriteBatch { texture, .. } => push(*texture),
        GpuCommand::ObjectBatch {
            texture,
            owner_texture,
            ..
        } => {
            push(*texture);
            owner_texture.iter().for_each(|id| push(*id));
        }
        GpuCommand::Landscape {
            base,
            liquid_mask,
            liquid,
            ..
        } => {
            push(*base);
            liquid_mask.iter().for_each(|id| push(*id));
            liquid.iter().for_each(|id| push(*id));
        }
        GpuCommand::Solid { .. } => {}
    }
    ids
}

fn physical_command_bounds(
    command: &GpuCommand,
    logical_extent: [u32; 2],
    presentation: GpuPresentation,
) -> Option<Rect> {
    let target = Rect::new(
        0,
        0,
        presentation.physical_extent[0],
        presentation.physical_extent[1],
    );
    if target.width == 0 || target.height == 0 {
        return None;
    }
    if !presentation.scale.is_finite()
        || presentation.scale <= 0.0
        || !presentation.world_zoom.is_finite()
    {
        return command.logical_bounds(logical_extent).map(|_| target);
    }
    let raster_padding = presentation.world_zoom.max(1.0 / presentation.scale);
    let logical_bounds =
        command.logical_bounds_with_raster_padding(logical_extent, raster_padding)?;
    let logical_clip = command
        .clip()
        .unwrap_or_else(|| Rect::new(0, 0, logical_extent[0], logical_extent[1]));
    let viewport_height = ((logical_extent[1] as f32) * presentation.scale)
        .ceil()
        .clamp(0.0, u32::MAX as f32) as u32;
    let projection_height = viewport_height.saturating_sub(presentation.crop_top);
    let projection = ClipperProjection::new(
        presentation.scale,
        (logical_extent[0], logical_extent[1]),
        projection_height,
        logical_clip,
    );
    projected_rect(logical_bounds, projection)?.intersection(target)
}

fn projected_rect(rect: Rect, projection: ClipperProjection) -> Option<Rect> {
    let (left, top) = projection.logical_to_physical(f64::from(rect.x), f64::from(rect.y));
    let (right, bottom) = projection.logical_to_physical(
        (i64::from(rect.x) + i64::from(rect.width)) as f64,
        (i64::from(rect.y) + i64::from(rect.height)) as f64,
    );
    if ![left, top, right, bottom].into_iter().all(f64::is_finite) {
        return None;
    }
    let clamp = |value: f64| value.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let physical_left = clamp(left.min(right).floor());
    let physical_top = clamp(top.min(bottom).floor());
    let physical_right = clamp(left.max(right).ceil()).max(physical_left);
    let physical_bottom = clamp(top.max(bottom).ceil()).max(physical_top);
    Some(Rect::new(
        physical_left,
        physical_top,
        physical_right.saturating_sub(physical_left) as u32,
        physical_bottom.saturating_sub(physical_top) as u32,
    ))
}

fn contains_rect(outer: Rect, inner: Rect) -> bool {
    let right = |rect: Rect| i64::from(rect.x) + i64::from(rect.width);
    let bottom = |rect: Rect| i64::from(rect.y) + i64::from(rect.height);
    outer.x <= inner.x
        && outer.y <= inner.y
        && right(outer) >= right(inner)
        && bottom(outer) >= bottom(inner)
}

fn contains_rect_with_raster_halo(owner: Rect, command: Rect) -> bool {
    if contains_rect(owner, command) {
        return true;
    }
    let right = |rect: Rect| i64::from(rect.x) + i64::from(rect.width);
    let bottom = |rect: Rect| i64::from(rect.y) + i64::from(rect.height);
    i64::from(command.x) >= i64::from(owner.x) - 1
        && i64::from(command.y) >= i64::from(owner.y) - 1
        && right(command) <= right(owner) + 1
        && bottom(command) <= bottom(owner) + 1
}

fn command_raster_halo_is_owned_by(owner: Rect, bounds: Rect, command: &GpuCommand) -> bool {
    !contains_rect(owner, bounds)
        && matches!(
            command,
            GpuCommand::Solid {
                topology: GpuPrimitiveTopology::LineList | GpuPrimitiveTopology::PointList,
                ..
            }
        )
        && contains_rect_with_raster_halo(owner, bounds)
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5")
))]
mod tests {
    use super::*;
    use clonk_graphics::{
        GammaRamp, GpuBlend, GpuGammaLut, GpuObjectSprite, GpuOuterModulation, GpuSampler,
        GpuScene, GpuSceneCaptureStats, GpuSolidAlphaMode, GpuSolidOuterModulation, GpuSolidStyle,
        GpuSolidVertex, GpuSpriteQuad, GpuTextureResource, GpuVertex,
    };
    use std::sync::Arc;

    fn vertex(x: f32, y: f32) -> GpuVertex {
        GpuVertex::new([x, y, 1.0], [0.0, 0.0], [1.0, 1.0, 1.0, 0.0])
    }

    fn quad(texture: GpuTextureId, bounds: Rect, clip: Option<Rect>) -> GpuCommand {
        let left = bounds.x as f32;
        let top = bounds.y as f32;
        let right = left + bounds.width as f32;
        let bottom = top + bounds.height as f32;
        GpuCommand::Quad {
            texture,
            owner_mask: None,
            vertices: [
                vertex(left, top),
                vertex(right, top),
                vertex(left, bottom),
                vertex(right, bottom),
            ],
            clip,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: false,
        }
    }

    fn texture(id: GpuTextureId, revision: u64) -> GpuTextureResource {
        let mut resource =
            GpuTextureResource::immutable_rgba(id, 1, 1, Arc::from([255_u8, 255, 255, 255]));
        resource.revision = revision;
        resource
    }

    fn scene(
        logical_extent: [u32; 2],
        textures: Vec<GpuTextureResource>,
        commands: Vec<GpuCommand>,
    ) -> GpuScene {
        GpuScene::new(
            logical_extent,
            Color::opaque(3, 5, 7),
            GpuGammaLut::from_ramp(&GammaRamp::identity()),
            GpuGammaMode::Fragment,
            textures,
            commands,
        )
    }

    fn frame(scene: GpuScene, presentation: GpuPresentation) -> RetainedGpuFrame {
        RetainedGpuFrame {
            layers: vec![RetainedGpuFrameLayer {
                scene,
                presentation,
                owner: None,
            }],
            capture_stats: GpuSceneCaptureStats::default(),
            physical_damage: None,
        }
    }

    #[test]
    fn moving_command_damages_old_and_new_physical_bounds() {
        let texture_id = GpuTextureId::fresh();
        let presentation = GpuPresentation::identity(80, 40);
        let previous = retained_gpu_frame_paint_owners(
            &frame(
                scene(
                    [80, 40],
                    vec![texture(texture_id, 1)],
                    vec![quad(texture_id, Rect::new(5, 7, 10, 8), None)],
                ),
                presentation,
            ),
            &[],
        );
        let current = retained_gpu_frame_paint_owners(
            &frame(
                scene(
                    [80, 40],
                    vec![texture(texture_id, 1)],
                    vec![quad(texture_id, Rect::new(35, 7, 10, 8), None)],
                ),
                presentation,
            ),
            &[],
        );

        assert_ne!(previous.nodes[0].id(), current.nodes[0].id());
        assert_ne!(previous.nodes[0].visual(), current.nodes[0].visual());
        assert_eq!(
            current.damage_from(&previous).rects(),
            &[Rect::new(5, 7, 10, 8), Rect::new(35, 7, 10, 8)]
        );
    }

    #[test]
    fn same_kind_insertion_does_not_rename_later_generic_atoms() {
        let texture_id = GpuTextureId::fresh();
        let presentation = GpuPresentation::identity(80, 40);
        let first = quad(texture_id, Rect::new(5, 5, 5, 5), None);
        let inserted = quad(texture_id, Rect::new(20, 5, 5, 5), None);
        let last = quad(texture_id, Rect::new(40, 5, 5, 5), None);
        let baseline = retained_gpu_frame_paint_owners(
            &frame(
                scene(
                    [80, 40],
                    vec![texture(texture_id, 1)],
                    vec![first.clone(), last.clone()],
                ),
                presentation,
            ),
            &[],
        );
        let with_insertion = retained_gpu_frame_paint_owners(
            &frame(
                scene(
                    [80, 40],
                    vec![texture(texture_id, 1)],
                    vec![first, inserted, last],
                ),
                presentation,
            ),
            &[],
        );

        assert_eq!(baseline.nodes[0], with_insertion.nodes[0]);
        assert_eq!(baseline.nodes[1], with_insertion.nodes[2]);
        assert_eq!(
            with_insertion.damage_from(&baseline).rects(),
            &[Rect::new(20, 5, 5, 5)]
        );
    }

    #[test]
    fn semantic_owner_halo_does_not_hide_textured_pixels_outside_its_bounds() {
        let texture_id = GpuTextureId::fresh();
        let command = quad(texture_id, Rect::new(9, 10, 11, 10), None);
        let extraction = retained_gpu_frame_paint_owners(
            &frame(
                scene([40, 30], vec![texture(texture_id, 1)], vec![command]),
                GpuPresentation::identity(40, 30),
            ),
            &[Rect::new(10, 10, 10, 10)],
        );

        assert_eq!(extraction.nodes.len(), 1);
        assert_eq!(extraction.nodes[0].bounds(), Rect::new(9, 10, 11, 10));
    }

    #[test]
    fn semantic_bounds_do_not_absorb_an_unrelated_contained_command() {
        let texture_id = GpuTextureId::fresh();
        let command = quad(texture_id, Rect::new(12, 12, 6, 6), None);
        let extraction = retained_gpu_frame_paint_owners(
            &frame(
                scene([40, 30], vec![texture(texture_id, 1)], vec![command]),
                GpuPresentation::identity(40, 30),
            ),
            &[Rect::new(10, 10, 12, 12)],
        );

        assert_eq!(extraction.nodes.len(), 1);
        assert_eq!(extraction.nodes[0].bounds(), Rect::new(12, 12, 6, 6));
    }

    #[test]
    fn semantic_owner_does_not_absorb_an_unproven_line_raster_halo() {
        let line = GpuCommand::Solid {
            vertices: vec![
                GpuSolidVertex {
                    position: [10.0, 10.0, 1.0],
                    color: [1.0; 4],
                    outer_modulation: GpuSolidOuterModulation::Ignore,
                },
                GpuSolidVertex {
                    position: [19.0, 10.0, 1.0],
                    color: [1.0; 4],
                    outer_modulation: GpuSolidOuterModulation::Ignore,
                },
            ],
            topology: GpuPrimitiveTopology::LineList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::NONE,
        };
        let extraction = retained_gpu_frame_paint_owners(
            &frame(
                scene([40, 30], Vec::new(), vec![line]),
                GpuPresentation::identity(40, 30),
            ),
            &[Rect::new(10, 10, 10, 10)],
        );

        assert_eq!(extraction.nodes.len(), 1);
    }

    #[test]
    fn tooltip_layer_provenance_owns_its_conservative_line_raster_halo() {
        let line = GpuCommand::Solid {
            vertices: vec![
                GpuSolidVertex {
                    position: [10.0, 10.0, 1.0],
                    color: [1.0; 4],
                    outer_modulation: GpuSolidOuterModulation::Ignore,
                },
                GpuSolidVertex {
                    position: [19.0, 10.0, 1.0],
                    color: [1.0; 4],
                    outer_modulation: GpuSolidOuterModulation::Ignore,
                },
            ],
            topology: GpuPrimitiveTopology::LineList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::NONE,
        };
        let mut owned = frame(
            scene([40, 30], Vec::new(), vec![line]),
            GpuPresentation::identity(40, 30),
        );
        owned.layers[0].owner = Some(RetainedGpuLayerOwner::StartupElementTooltip);

        let extraction = retained_gpu_frame_paint_owners(&owned, &[Rect::new(10, 10, 10, 10)]);

        assert!(extraction.nodes.is_empty());
    }

    // Rust-only batching invariant: ordered native text may split a logical
    // command stream into more RetainedGpuFrameLayer values without changing
    // painter order or pixels.
    #[test]
    fn layer_boundary_changes_do_not_rename_or_invalidate_atoms() {
        let texture_id = GpuTextureId::fresh();
        let presentation = GpuPresentation::identity(80, 40);
        let first = quad(texture_id, Rect::new(5, 5, 5, 5), None);
        let last = quad(texture_id, Rect::new(40, 5, 5, 5), None);
        let baseline = retained_gpu_frame_paint_owners(
            &frame(
                scene(
                    [80, 40],
                    vec![texture(texture_id, 1)],
                    vec![first.clone(), last.clone()],
                ),
                presentation,
            ),
            &[],
        );
        let split = retained_gpu_frame_paint_owners(
            &RetainedGpuFrame {
                layers: vec![
                    RetainedGpuFrameLayer {
                        scene: scene([80, 40], vec![texture(texture_id, 1)], vec![first]),
                        presentation,
                        owner: None,
                    },
                    RetainedGpuFrameLayer {
                        scene: scene([80, 40], vec![texture(texture_id, 1)], vec![last]),
                        presentation,
                        owner: None,
                    },
                ],
                capture_stats: GpuSceneCaptureStats::default(),
                physical_damage: None,
            },
            &[],
        );

        assert_eq!(baseline.signature, split.signature);
        assert_eq!(baseline.nodes, split.nodes);
        assert!(split.damage_from(&baseline).rects().is_empty());
    }

    #[test]
    fn referenced_texture_revision_is_part_of_exact_visual_state() {
        let texture_id = GpuTextureId::fresh();
        let presentation = GpuPresentation::identity(40, 30);
        let command = quad(texture_id, Rect::new(4, 6, 8, 7), None);
        let previous = retained_gpu_frame_paint_owners(
            &frame(
                scene(
                    [40, 30],
                    vec![texture(texture_id, 8)],
                    vec![command.clone()],
                ),
                presentation,
            ),
            &[],
        );
        let current = retained_gpu_frame_paint_owners(
            &frame(
                scene([40, 30], vec![texture(texture_id, 9)], vec![command]),
                presentation,
            ),
            &[],
        );

        assert_eq!(previous.nodes[0].id(), current.nodes[0].id());
        assert_eq!(previous.nodes[0].bounds(), current.nodes[0].bounds());
        assert_ne!(previous.nodes[0].visual(), current.nodes[0].visual());
        assert_eq!(
            current.damage_from(&previous).rects(),
            &[Rect::new(4, 6, 8, 7)]
        );
    }

    #[test]
    fn sprite_object_and_solid_batches_split_at_primitive_boundaries() {
        let sprite_texture = GpuTextureId::fresh();
        let object_texture = GpuTextureId::fresh();
        let sprite = |left: f32| GpuSpriteQuad {
            rect: [left, 2.0, left + 3.0, 6.0],
            uv: [0.0, 0.0, 1.0, 1.0],
            modulation: 0x00ff_ffff,
        };
        let object = |left: f32| {
            GpuObjectSprite::new(
                [
                    [left, 10.0, 1.0],
                    [left + 3.0, 10.0, 1.0],
                    [left, 14.0, 1.0],
                    [left + 3.0, 14.0, 1.0],
                ],
                [0.0, 0.0, 1.0, 1.0],
                [0x00ff_ffff; 4],
                GpuSampler::Nearest,
                0.0,
                false,
                GpuOuterModulation::Inherit,
            )
        };
        let solid_vertex = |x: f32, y: f32| GpuSolidVertex {
            position: [x, y, 1.0],
            color: [1.0, 1.0, 1.0, 1.0],
            outer_modulation: GpuSolidOuterModulation::Ignore,
        };
        let commands = vec![
            GpuCommand::SpriteBatch {
                texture: sprite_texture,
                quads: vec![sprite(2.0), sprite(8.0)],
                clip: None,
                blend: GpuBlend::Normal,
                mod2: false,
                gamma: false,
                outer_modulation: GpuOuterModulation::Inherit,
            },
            GpuCommand::ObjectBatch {
                texture: object_texture,
                owner_texture: None,
                sprites: vec![object(2.0), object(8.0)],
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            },
            GpuCommand::Solid {
                vertices: vec![
                    solid_vertex(2.0, 18.0),
                    solid_vertex(5.0, 18.0),
                    solid_vertex(2.0, 21.0),
                    solid_vertex(8.0, 18.0),
                    solid_vertex(11.0, 18.0),
                    solid_vertex(8.0, 21.0),
                ],
                topology: GpuPrimitiveTopology::TriangleList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Normal,
                style: GpuSolidStyle::NONE,
            },
        ];
        let extraction = retained_gpu_frame_paint_owners(
            &frame(
                scene(
                    [30, 30],
                    vec![texture(sprite_texture, 1), texture(object_texture, 2)],
                    commands,
                ),
                GpuPresentation::identity(30, 30),
            ),
            &[],
        );

        assert_eq!(extraction.nodes.len(), 6);
        for node in &extraction.nodes[..2] {
            let GpuCommand::SpriteBatch { quads, .. } = &node.visual().command else {
                panic!("sprite atom retained a different command type");
            };
            assert_eq!(quads.len(), 1);
        }
        for node in &extraction.nodes[2..4] {
            let GpuCommand::ObjectBatch { sprites, .. } = &node.visual().command else {
                panic!("object atom retained a different command type");
            };
            assert_eq!(sprites.len(), 1);
        }
        for node in &extraction.nodes[4..] {
            let GpuCommand::Solid { vertices, .. } = &node.visual().command else {
                panic!("solid atom retained a different command type");
            };
            assert_eq!(vertices.len(), 3);
        }
    }

    #[test]
    fn fractional_primary_clip_projection_uses_rounded_clip_relative_viewport() {
        let texture_id = GpuTextureId::fresh();
        let clip = Rect::new(1, 1, 2, 2);
        let command = quad(texture_id, Rect::new(1, 1, 1, 1), Some(clip));
        let extraction = retained_gpu_frame_paint_owners(
            &frame(
                scene([4, 4], vec![texture(texture_id, 1)], vec![command.clone()]),
                GpuPresentation {
                    physical_extent: [6, 6],
                    scale: 1.5,
                    crop_top: 0,
                    world_zoom: 1.0,
                },
            ),
            &[],
        );

        assert_eq!(extraction.nodes[0].bounds(), Rect::new(1, 2, 2, 2));
        assert_eq!(extraction.nodes[0].visual().command, command);
        assert_eq!(extraction.nodes[0].visual().command.clip(), Some(clip));
    }

    #[test]
    fn semantic_owner_projection_uses_the_full_target_and_clips_cropped_output() {
        let bounds = retained_gpu_physical_bounds(
            Rect::new(1, 1, 2, 2),
            [4, 4],
            GpuPresentation {
                physical_extent: [6, 4],
                scale: 1.5,
                crop_top: 2,
                world_zoom: 1.0,
            },
        );

        assert_eq!(bounds, Some(Rect::new(1, 0, 4, 3)));
    }

    #[test]
    fn frame_signature_changes_force_full_physical_damage() {
        let presentation = GpuPresentation::identity(20, 12);
        let baseline_scene = scene([20, 12], Vec::new(), Vec::new());
        let baseline =
            retained_gpu_frame_paint_owners(&frame(baseline_scene.clone(), presentation), &[]);

        let mut clear = baseline_scene.clone();
        clear.clear = Color::opaque(9, 5, 7);
        let mut gamma_revision = baseline_scene.clone();
        gamma_revision.gamma.revision = gamma_revision.gamma.revision.wrapping_add(1);
        let mut gamma_mode = baseline_scene.clone();
        gamma_mode.gamma_mode = GpuGammaMode::Monitor;
        let mut logical_extent = baseline_scene.clone();
        logical_extent.logical_extent = [19, 12];
        let changed = [
            retained_gpu_frame_paint_owners(&frame(clear, presentation), &[]),
            retained_gpu_frame_paint_owners(&frame(gamma_revision, presentation), &[]),
            retained_gpu_frame_paint_owners(&frame(gamma_mode, presentation), &[]),
            retained_gpu_frame_paint_owners(&frame(logical_extent, presentation), &[]),
            retained_gpu_frame_paint_owners(
                &frame(
                    baseline_scene,
                    GpuPresentation {
                        scale: 1.5,
                        ..presentation
                    },
                ),
                &[],
            ),
        ];

        for extraction in changed {
            assert_ne!(baseline.signature, extraction.signature);
            assert_eq!(
                extraction.damage_from(&baseline).rects(),
                &[Rect::new(0, 0, 20, 12)]
            );
        }
    }
}
