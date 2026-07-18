use crate::{Group, GroupError};
use image::{self, DynamicImage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    index: HashMap<String, PathBuf>,
    cache: Mutex<HashMap<String, Arc<GraphicsImage>>>,
}

impl GraphicsResource {
    pub fn from_group(group: Group) -> Result<Self, GraphicsError> {
        let mut index = HashMap::new();
        collect_entries(&group, PathBuf::new(), &mut index)?;
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
        let key = normalize_key(name);
        self.index.contains_key(&key)
    }

    pub fn load_image(&self, name: &str) -> Result<GraphicsImage, GraphicsError> {
        let key = normalize_key(name);
        let path = self
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

        let data = self.group.read_file(path)?;
        let image = decode_image(&data).map_err(|source| GraphicsError::ImageDecode {
            name: path.display().to_string(),
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

fn collect_entries(
    group: &Group,
    base: PathBuf,
    index: &mut HashMap<String, PathBuf>,
) -> Result<(), GroupError> {
    for entry in group.entries()? {
        let mut relative = base.clone();
        relative.push(&entry.relative_path);
        if entry.is_directory {
            let child = group.open_child(&entry.relative_path)?;
            collect_entries(&child, relative, index)?;
        } else {
            insert_index(index, &relative);
        }
    }
    Ok(())
}

fn insert_index(index: &mut HashMap<String, PathBuf>, path: &Path) {
    let normalized = normalize_key(path);
    index
        .entry(normalized.clone())
        .or_insert_with(|| path.to_path_buf());
    if let Some(name) = path.file_name().and_then(|os| os.to_str()) {
        let lower = name.to_ascii_lowercase();
        index.entry(lower).or_insert_with(|| path.to_path_buf());
    }
}

fn normalize_key(path: impl AsRef<Path>) -> String {
    path_as_segments(path.as_ref())
        .iter()
        .map(|segment| segment.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

fn path_as_segments(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

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
}
