//! Resource loading for classic `Particle.txt` newgfx definitions.

use crate::definition::{
    create_ini_name_tree, parse_i32, parse_int_array, parse_int_array_with_default,
};
use crate::{GraphicsImage, Group, GroupEntry, GroupError};
use std::collections::HashSet;
use std::path::PathBuf;
use thiserror::Error;

const C4PX_MAX_PARTICLE: i32 = 256;

/// The fields compiled by `C4ParticleDefCore::CompileFunc`.
///
/// `placement` is retained because it is part of the native core, but legacy
/// `Particle.txt` never compiles it and therefore always leaves it at zero.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ParticleDefinitionCore {
    pub name: String,
    pub max_count: i32,
    pub min_lifetime: i32,
    pub max_lifetime: i32,
    pub y_off: i32,
    pub delay: i32,
    pub repeats: i32,
    pub reverse: i32,
    pub fade_out_len: i32,
    pub fade_out_delay: i32,
    pub r_by_v: i32,
    pub placement: i32,
    pub gravity_acc: i32,
    pub wind_drift: i32,
    pub vertex_count: i32,
    pub vertex_y: i32,
    pub additive: i32,
    pub attach: i32,
    pub alpha_fade: i32,
    pub parallaxity: [i32; 2],
    pub init_fn: String,
    pub exec_fn: String,
    pub draw_fn: String,
    pub collision_fn: String,
}

impl Default for ParticleDefinitionCore {
    fn default() -> Self {
        Self {
            name: String::new(),
            max_count: C4PX_MAX_PARTICLE,
            min_lifetime: 0,
            max_lifetime: 0,
            y_off: 0,
            delay: 0,
            repeats: 0,
            reverse: 0,
            fade_out_len: 0,
            fade_out_delay: 0,
            r_by_v: 0,
            placement: 0,
            gravity_acc: 0,
            wind_drift: 0,
            vertex_count: 0,
            vertex_y: 0,
            additive: 0,
            attach: 0,
            alpha_fade: 0,
            parallaxity: [100, 100],
            init_fn: String::new(),
            exec_fn: String::new(),
            draw_fn: String::new(),
            collision_fn: String::new(),
        }
    }
}

/// Source and target rectangle for one particle animation phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ParticleFacet {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

/// A parsed particle core paired with its immediate `Graphics.png` surface.
#[derive(Debug, Clone)]
pub struct ParticleDefinition {
    pub core: ParticleDefinitionCore,
    pub image: GraphicsImage,
    pub facet: ParticleFacet,
}

#[derive(Debug, Error)]
pub enum ParticleDefinitionError {
    #[error("particle core `Particle.txt` missing")]
    ParticleCoreMissing,
    #[error("particle graphics `Graphics.png` missing")]
    GraphicsMissing,
    #[error("particle graphics `{path}` could not be decoded: {source}")]
    GraphicsDecode {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("particle graphics dimensions {width}x{height} are not supported")]
    InvalidGraphicsDimensions { width: u32, height: u32 },
    #[error("particle facet dimensions {width}x{height} are not valid")]
    InvalidFacetDimensions { width: i32, height: i32 },
    #[error(transparent)]
    Group(#[from] GroupError),
}

impl ParticleDefinition {
    /// Load the exact immediate `Particle.txt` and `Graphics.png` entries.
    /// Child groups and alternative image formats are deliberately ignored.
    pub fn load(group: &Group) -> Result<Self, ParticleDefinitionError> {
        let entries = group.entries()?;
        let core_entry = find_immediate_entry(&entries, b"Particle.txt")
            .ok_or(ParticleDefinitionError::ParticleCoreMissing)?;
        let source = group.read_entry_bytes_exact_cow(core_entry)?;
        let (core, raw_facet) = parse_particle_core(source.as_ref());

        let graphics_entry = find_immediate_entry(&entries, b"Graphics.png")
            .ok_or(ParticleDefinitionError::GraphicsMissing)?;
        let graphics_bytes = group.read_entry_bytes_exact_cow(graphics_entry)?;
        let rgba =
            image::load_from_memory_with_format(graphics_bytes.as_ref(), image::ImageFormat::Png)
                .map_err(|source| ParticleDefinitionError::GraphicsDecode {
                    path: graphics_entry.relative_path.clone(),
                    source,
                })?
                .into_rgba8();
        let (image_width, image_height) = rgba.dimensions();
        let facet = normalize_facet(raw_facet, image_width, image_height)?;
        let image = GraphicsImage::new(image_width, image_height, rgba.into_raw());

        Ok(Self { core, image, facet })
    }
}

fn find_immediate_entry<'a>(entries: &'a [GroupEntry], name: &[u8]) -> Option<&'a GroupEntry> {
    entries
        .iter()
        .find(|entry| entry.name_bytes.eq_ignore_ascii_case(name))
}

fn parse_particle_core(source: &[u8]) -> (ParticleDefinitionCore, ParticleFacet) {
    let source = source.split(|byte| *byte == 0).next().unwrap_or_default();
    let text = clonk_script::c4_string_from_bytes(source);
    let nodes = create_ini_name_tree(&text);
    let particle_node = nodes
        .iter()
        .position(|node| node.parent == 0 && node.name == "Particle");
    let mut core = ParticleDefinitionCore::default();
    let mut facet = ParticleFacet::default();
    let mut seen = HashSet::new();

    for node in nodes
        .iter()
        .filter(|node| Some(node.parent) == particle_node)
    {
        if !seen.insert(node.name) {
            continue;
        }
        let string_value = node.raw_value.trim_start_matches([' ', '\t']);
        let int_value = |default| parse_i32(node.raw_value).unwrap_or(default);
        match node.name {
            "Name" => core.name = string_value.to_string(),
            "MaxCount" => core.max_count = int_value(C4PX_MAX_PARTICLE),
            "MinLifetime" => core.min_lifetime = int_value(0),
            "MaxLifetime" => core.max_lifetime = int_value(0),
            "InitFn" => core.init_fn = string_value.to_string(),
            "ExecFn" => core.exec_fn = string_value.to_string(),
            "CollisionFn" => core.collision_fn = string_value.to_string(),
            "DrawFn" => core.draw_fn = string_value.to_string(),
            "Face" => {
                let mut values = parse_int_array(node.raw_value);
                facet = ParticleFacet {
                    x: values.next().unwrap_or(0),
                    y: values.next().unwrap_or(0),
                    width: values.next().unwrap_or(0),
                    height: values.next().unwrap_or(0),
                    target_x: values.next().unwrap_or(0),
                    target_y: values.next().unwrap_or(0),
                };
            }
            "YOff" => core.y_off = int_value(0),
            "Delay" => core.delay = int_value(0),
            "Repeats" => core.repeats = int_value(0),
            "Reverse" => core.reverse = int_value(0),
            "FadeOutLen" => core.fade_out_len = int_value(0),
            "FadeOutDelay" => core.fade_out_delay = int_value(0),
            "RByV" => core.r_by_v = int_value(0),
            "GravityAcc" => core.gravity_acc = int_value(0),
            "WindDrift" => core.wind_drift = int_value(0),
            "VertexCount" => core.vertex_count = int_value(0),
            "VertexY" => core.vertex_y = int_value(0),
            "Additive" => core.additive = int_value(0),
            "AlphaFade" => core.alpha_fade = int_value(0),
            "Parallaxity" => {
                let mut values = parse_int_array_with_default(node.raw_value, 100);
                core.parallaxity = [values.next().unwrap_or(100), values.next().unwrap_or(100)];
            }
            "Attach" => core.attach = int_value(0),
            _ => {}
        }
    }

    (core, facet)
}

fn normalize_facet(
    facet: ParticleFacet,
    image_width: u32,
    image_height: u32,
) -> Result<ParticleFacet, ParticleDefinitionError> {
    let whole_width = i32::try_from(image_width).map_err(|_| {
        ParticleDefinitionError::InvalidGraphicsDimensions {
            width: image_width,
            height: image_height,
        }
    })?;
    let whole_height = i32::try_from(image_height).map_err(|_| {
        ParticleDefinitionError::InvalidGraphicsDimensions {
            width: image_width,
            height: image_height,
        }
    })?;
    if whole_width == 0 || whole_height == 0 {
        return Err(ParticleDefinitionError::InvalidGraphicsDimensions {
            width: image_width,
            height: image_height,
        });
    }
    if facet.width == 0 {
        return Ok(ParticleFacet {
            width: whole_width,
            height: whole_height,
            ..ParticleFacet::default()
        });
    }

    // Native accepts signed offsets and facets extending beyond the image.
    // Reject only unsafe divisors; phase-count validation remains an engine
    // concern and reproduces C4ParticleDef::Load's zero-length rejection.
    if facet.width <= 0 || facet.height <= 0 {
        return Err(ParticleDefinitionError::InvalidFacetDimensions {
            width: facet.width,
            height: facet.height,
        });
    }
    Ok(facet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_png(path: &std::path::Path, width: u32, height: u32, color: [u8; 4]) {
        image::RgbaImage::from_pixel(width, height, image::Rgba(color))
            .save(path)
            .expect("write PNG fixture");
    }

    #[test]
    fn loads_compiled_fields_with_native_name_tree_rules() {
        let temp = tempdir().expect("tempdir");
        let group_path = temp.path().join("Smoke.c4d");
        fs::create_dir_all(&group_path).expect("particle group");
        fs::write(
            group_path.join("Particle.txt"),
            b"[Particle]\nName= Smoke \nName=Ignored\nMaxCount=17\nMinLifetime=2\nMaxLifetime=30\nInitFn=StdInit\nExecFn=StdExec\nCollisionFn=Bounce\nDrawFn=Std\nFace=0,0,2,4,-1,-2\nYOff=3\nDelay=4\nRepeats=5\nReverse=6\nFadeOutLen=7\nFadeOutDelay=8\nRByV=9\nGravityAcc=10\nWindDrift=11\nVertexCount=12\nVertexY=13\nAdditive=14\nAlphaFade=15\nParallaxity=80\nAttach=16\nPlacement=99\0Name=AfterNul\n",
        )
        .expect("Particle.txt");
        write_png(&group_path.join("gRaPhIcS.PnG"), 8, 4, [1, 2, 3, 255]);

        let definition = ParticleDefinition::load(&Group::open(&group_path).expect("group"))
            .expect("particle definition");
        assert_eq!(definition.core.name, "Smoke ");
        assert_eq!(definition.core.max_count, 17);
        assert_eq!(definition.core.min_lifetime, 2);
        assert_eq!(definition.core.max_lifetime, 30);
        assert_eq!(definition.core.init_fn, "StdInit");
        assert_eq!(definition.core.exec_fn, "StdExec");
        assert_eq!(definition.core.collision_fn, "Bounce");
        assert_eq!(definition.core.draw_fn, "Std");
        assert_eq!(definition.core.y_off, 3);
        assert_eq!(definition.core.delay, 4);
        assert_eq!(definition.core.repeats, 5);
        assert_eq!(definition.core.reverse, 6);
        assert_eq!(definition.core.fade_out_len, 7);
        assert_eq!(definition.core.fade_out_delay, 8);
        assert_eq!(definition.core.r_by_v, 9);
        assert_eq!(definition.core.placement, 0, "Placement is not compiled");
        assert_eq!(definition.core.gravity_acc, 10);
        assert_eq!(definition.core.wind_drift, 11);
        assert_eq!(definition.core.vertex_count, 12);
        assert_eq!(definition.core.vertex_y, 13);
        assert_eq!(definition.core.additive, 14);
        assert_eq!(definition.core.alpha_fade, 15);
        assert_eq!(definition.core.parallaxity, [80, 100]);
        assert_eq!(definition.core.attach, 16);
        assert_eq!(
            definition.facet,
            ParticleFacet {
                x: 0,
                y: 0,
                width: 2,
                height: 4,
                target_x: -1,
                target_y: -2,
            }
        );
    }

    #[test]
    fn defaults_whole_surface_facet_and_ignores_nested_graphics() {
        let temp = tempdir().expect("tempdir");
        let group_path = temp.path().join("Particle.c4d");
        let child_path = group_path.join("Child.c4d");
        fs::create_dir_all(&child_path).expect("nested group");
        fs::write(group_path.join("Particle.txt"), b"[Particle]\n").expect("Particle.txt");
        write_png(&child_path.join("Graphics.png"), 9, 9, [9, 9, 9, 255]);

        let group = Group::open(&group_path).expect("group");
        assert!(matches!(
            ParticleDefinition::load(&group),
            Err(ParticleDefinitionError::GraphicsMissing)
        ));

        write_png(&group_path.join("Graphics.png"), 3, 2, [3, 2, 1, 255]);
        let definition = ParticleDefinition::load(&Group::open(&group_path).expect("reopen"))
            .expect("particle definition");
        assert_eq!(definition.core, ParticleDefinitionCore::default());
        assert_eq!(definition.image.width(), 3);
        assert_eq!(definition.image.height(), 2);
        assert_eq!(
            definition.facet,
            ParticleFacet {
                width: 3,
                height: 2,
                ..ParticleFacet::default()
            }
        );
    }

    #[test]
    fn preserves_native_out_of_bounds_facet_but_rejects_unsafe_divisors() {
        let temp = tempdir().expect("tempdir");
        let group_path = temp.path().join("Particle.c4d");
        fs::create_dir_all(&group_path).expect("particle group");
        fs::write(
            group_path.join("Particle.txt"),
            b"[Particle]\nFace=-5,-6,5,2,-2,-3\n",
        )
        .expect("Particle.txt");
        write_png(&group_path.join("Graphics.png"), 4, 1, [0, 0, 0, 255]);

        let definition = ParticleDefinition::load(&Group::open(&group_path).expect("group"))
            .expect("native permits an out-of-bounds positive facet");
        assert_eq!(
            definition.facet,
            ParticleFacet {
                x: -5,
                y: -6,
                width: 5,
                height: 2,
                target_x: -2,
                target_y: -3,
            }
        );

        fs::write(
            group_path.join("Particle.txt"),
            b"[Particle]\nFace=-5,-6,5,0,-2,-3\n",
        )
        .expect("invalid Particle.txt");

        assert!(matches!(
            ParticleDefinition::load(&Group::open(&group_path).expect("group")),
            Err(ParticleDefinitionError::InvalidFacetDimensions { .. })
        ));
    }
}
