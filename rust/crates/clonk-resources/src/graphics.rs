use crate::{Group, GroupEntry, GroupError};
use image::{self, DynamicImage};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphicsError {
    #[error("graphics entry `{name}` not found")]
    EntryNotFound { name: String },
    #[error("failed to decode image `{name}`: {source}")]
    ImageDecode {
        name: String,
        #[source]
        source: image::ImageError,
    },
    #[error(transparent)]
    Group(#[from] GroupError),
}

#[derive(Debug, Clone)]
pub struct GraphicsImage {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl GraphicsImage {
    pub fn new(width: u32, height: u32, mut pixels: Vec<u8>) -> Self {
        blacken_fully_transparent_rgba(&mut pixels);
        Self {
            width,
            height,
            pixels: Arc::from(pixels.into_boxed_slice()),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn clone_pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }

    pub fn into_parts(self) -> (u32, u32, Arc<[u8]>) {
        (self.width, self.height, self.pixels)
    }
}

/// `C4Surface` canonicalizes fully-transparent pixels to transparent black
/// while loading them (`C4Surface.cpp:733,972,982`). Rust RGBA uses opacity
/// rather than C4's inverted transparency byte, so C4 alpha `0xff` is alpha
/// zero here. Partial-alpha and opaque pixels retain their original RGB.
pub(crate) fn blacken_fully_transparent_rgba(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            pixel[..3].fill(0);
        }
    }
}

pub struct GraphicsResource {
    group: Group,
    index: HashMap<Vec<u8>, GroupEntry>,
    cache: Mutex<HashMap<Vec<u8>, Arc<GraphicsImage>>>,
}

impl GraphicsResource {
    pub fn from_group(group: Group) -> Result<Self, GraphicsError> {
        let mut index = HashMap::new();
        for entry in group.entries()? {
            index
                .entry(fold_ascii_case(&entry.name_bytes))
                .or_insert(entry);
        }
        Ok(Self {
            group,
            index,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, GraphicsError> {
        let group = Group::open(path)?;
        Self::from_group(group)
    }

    pub fn contains(&self, name: &str) -> bool {
        let key = fold_ascii_case(&clonk_script::c4_string_bytes(name));
        self.index.contains_key(&key)
    }

    pub fn load_image(&self, name: &str) -> Result<GraphicsImage, GraphicsError> {
        let key = fold_ascii_case(&clonk_script::c4_string_bytes(name));
        let entry = self
            .index
            .get(&key)
            .ok_or_else(|| GraphicsError::EntryNotFound {
                name: name.to_string(),
            })?;

        if let Some(cached) = self
            .cache
            .lock()
            .expect("graphics cache poisoned")
            .get(&key)
        {
            return Ok((**cached).clone());
        }

        let data = self.group.read_entry_bytes_exact(entry)?;
        let image = decode_image(&data).map_err(|source| GraphicsError::ImageDecode {
            name: String::from_utf8_lossy(&entry.name_bytes).into_owned(),
            source,
        })?;
        let rgba = image.into_rgba8();
        let (width, height) = rgba.dimensions();
        let pixels = rgba.into_raw();
        let graphics_image = Arc::new(GraphicsImage::new(width, height, pixels));
        self.cache
            .lock()
            .expect("graphics cache poisoned")
            .insert(key.clone(), Arc::clone(&graphics_image));
        Ok((*graphics_image).clone())
    }
}

fn decode_image(bytes: &[u8]) -> Result<DynamicImage, image::ImageError> {
    image::load_from_memory(bytes)
}

fn fold_ascii_case(name: &[u8]) -> Vec<u8> {
    let mut folded = name.to_vec();
    folded.make_ascii_lowercase();
    folded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn solid_png(pixel: [u8; 4]) -> Vec<u8> {
        let image =
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba(pixel)));
        let mut bytes = Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageOutputFormat::Png)
            .expect("encode PNG");
        bytes.into_inner()
    }

    #[test]
    fn loads_png_image_from_directory() {
        let dir = tempdir().expect("tempdir");
        let graphics_dir = dir.path().join("Graphics.c4g");
        fs::create_dir_all(&graphics_dir).expect("create dir");
        let image_path = graphics_dir.join("Example.png");
        let png_data = include_bytes!("../../../../planet/Graphics.c4g/StartupBigButton.png");
        fs::write(&image_path, png_data).expect("write png");

        let group = Group::open(&graphics_dir).expect("group");
        let resource = GraphicsResource::from_group(group).expect("resource");
        let image = resource.load_image("Example.png").expect("image");

        assert_eq!(image.width(), 224);
        assert_eq!(image.height(), 40);
        assert_eq!(image.pixels().len(), 224 * 40 * 4);
    }

    #[test]
    fn loaded_images_blacken_only_fully_transparent_rgb() {
        let dir = tempdir().expect("tempdir");
        let graphics_dir = dir.path().join("Graphics.c4g");
        fs::create_dir_all(&graphics_dir).expect("create dir");
        let image_path = graphics_dir.join("Transparent.png");
        image::RgbaImage::from_raw(
            3,
            1,
            vec![255, 127, 63, 0, 200, 100, 50, 1, 10, 20, 30, 255],
        )
        .expect("rgba image")
        .save(&image_path)
        .expect("write png");

        let group = Group::open(&graphics_dir).expect("group");
        let resource = GraphicsResource::from_group(group).expect("resource");
        let image = resource.load_image("Transparent.png").expect("image");

        assert_eq!(
            image.pixels(),
            &[0, 0, 0, 0, 200, 100, 50, 1, 10, 20, 30, 255]
        );
        assert_eq!(
            resource
                .load_image("transparent.png")
                .expect("cached image")
                .pixels(),
            image.pixels(),
            "cache hits retain the canonical loaded surface"
        );
    }

    #[test]
    fn graphics_lookup_is_top_level_and_ignores_unrelated_opaque_names() {
        let mut nested = crate::MutableGroup::new_bytes(b"Nested.c4g".to_vec());
        nested
            .add_file("Player.png", solid_png([200, 10, 20, 255]))
            .expect("nested collision");
        nested
            .add_file("OnlyNested.png", solid_png([210, 30, 40, 255]))
            .expect("nested-only image");

        let mut root = crate::MutableGroup::new_bytes(b"Fixture.bin".to_vec());
        root.add_child("Nested.c4g", nested)
            .expect("nested graphics group");
        root.add_packed_child_bytes_with_metadata(
            b"Broken.c4g".to_vec(),
            b"not a C4Group".to_vec(),
            0,
            1,
            false,
        )
        .expect("malformed child");
        root.add_file_bytes_with_metadata(
            b"Opaque\xff.bin".to_vec(),
            b"unrelated".to_vec(),
            1,
            false,
        )
        .expect("opaque sibling");
        root.add_file("pLaYeR.PnG", solid_png([10, 20, 30, 255]))
            .expect("top-level image");

        let group = Group::from_raw_memory(
            PathBuf::from("Fixture.bin"),
            root.pack_raw().expect("pack graphics fixture"),
        )
        .expect("open graphics fixture");
        let resource = GraphicsResource::from_group(group)
            .expect("unrelated malformed child and opaque name are not opened or decoded");

        assert!(resource.contains("PLAYER.PNG"));
        assert!(!resource.contains("Nested.c4g/Player.png"));
        assert!(!resource.contains("OnlyNested.png"));
        assert_eq!(
            resource
                .load_image("player.png")
                .expect("case-insensitive top-level image")
                .pixels(),
            [10, 20, 30, 255]
        );
        assert!(matches!(
            resource.load_image("OnlyNested.png"),
            Err(GraphicsError::EntryNotFound { .. })
        ));

        let mut matching_child = crate::MutableGroup::new_bytes(b"Fixture.bin".to_vec());
        matching_child
            .add_packed_child_with_metadata("Player.png", b"not an image".to_vec(), 0, 1, false)
            .expect("matching child entry");
        let matching_child = Group::from_raw_memory(
            PathBuf::from("Fixture.bin"),
            matching_child
                .pack_raw()
                .expect("pack matching child fixture"),
        )
        .expect("open matching child fixture");
        let resource = GraphicsResource::from_group(matching_child).expect("index matching child");
        assert!(matches!(
            resource.load_image("Player.png"),
            Err(GraphicsError::ImageDecode { .. })
        ));
    }
}
